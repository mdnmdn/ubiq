//! Phase 7 in one file: a listening drone's state file, discovering it, adopting it instead of
//! starting a rival, and stopping it by name.
//!
//! Everything here drives the real `ubiq-drone` binary, over its real socket and its real state
//! file, for the same reason `detach.rs` does: adoption and staleness are facts about a process
//! and a filesystem, and a test that stayed in `Relay` would prove neither.
#![cfg(unix)]

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ubiq_drone::state::DroneState;

/// How long a test waits for a slow drone to bind, stop or write its state.
const PATIENCE: Duration = Duration::from_secs(20);

/// A drone process listening on a socket of this test's own.
struct Drone {
    child: Child,
    socket: PathBuf,
    /// The socket and the project folder both live here, so nothing this test binds outlives it.
    _home: tempfile::TempDir,
    project: PathBuf,
}

impl Drop for Drone {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drone {
    /// Start one, and return once it has bound. `--foreground` is what a multiplexer would run:
    /// the test holds the process itself, because what is under test is the socket and the state
    /// file next to it, never the thing holding the process.
    fn listening(linger: &str) -> Self {
        let home = tempfile::tempdir().expect("a home for this test");
        let project = home.path().join("project");
        std::fs::create_dir_all(&project).expect("the project folder");
        let socket = home.path().join("drone.sock");

        let child = spawn_listen(&socket, &project, linger);

        let deadline = Instant::now() + PATIENCE;
        while Instant::now() < deadline && UnixStream::connect(&socket).is_err() {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(
            UnixStream::connect(&socket).is_ok(),
            "the drone never bound its socket"
        );

        Self {
            child,
            socket,
            _home: home,
            project,
        }
    }

    fn state_path(&self) -> PathBuf {
        ubiq_drone::state::state_path(&self.socket)
    }

    /// Wait for the state file to exist and parse, up to [`PATIENCE`]: it is written just after
    /// `bind`, but that is still a race against this test's own read.
    fn state(&self) -> DroneState {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Ok(state) = DroneState::read(&self.state_path()) {
                return state;
            }
            assert!(
                Instant::now() < deadline,
                "the drone never wrote its state file"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Whether the process has ended, waiting up to `patience` for it to.
    fn ended_within(&mut self, patience: Duration) -> bool {
        let deadline = Instant::now() + patience;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return true,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => return false,
            }
        }
        false
    }
}

fn spawn_listen(socket: &std::path::Path, project: &std::path::Path, linger: &str) -> Child {
    Command::new(env!("CARGO_BIN_EXE_ubiq-drone"))
        .arg("--listen")
        .arg(socket)
        .arg("--foreground")
        .args(["--linger", linger])
        .arg("--root")
        .arg(project)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("starting a drone")
}

/// A listening drone writes a state file naming its pid, its roots and its pane count, and
/// `--list` finds it.
#[test]
fn a_listening_drone_is_found_by_list() {
    let drone = Drone::listening("60");
    let state = drone.state();

    assert_eq!(state.pid, drone.child.id());
    assert_eq!(state.panes, 0);
    assert_eq!(state.socket, drone.socket.to_string_lossy());
    assert!(
        state
            .roots
            .iter()
            .any(|root| std::path::Path::new(root) == drone.project),
        "the state file names the root this drone was launched with"
    );

    let listed = ubiq_drone::state::list(drone.socket.parent().expect("the socket's directory"));
    assert!(
        listed.iter().any(|found| found.pid == state.pid),
        "--list finds a drone that is actually listening"
    );
}

/// A second `--listen` for the same socket adopts the running drone rather than starting a rival:
/// it exits `0` having served nothing, and the original process is still the one holding the
/// socket.
#[test]
fn a_second_listen_adopts_instead_of_racing() {
    let drone = Drone::listening("60");
    let first_pid = drone.state().pid;

    let adopting = Command::new(env!("CARGO_BIN_EXE_ubiq-drone"))
        .arg("--listen")
        .arg(&drone.socket)
        .arg("--foreground")
        .arg("--root")
        .arg(&drone.project)
        .output()
        .expect("running the adopting drone");

    assert!(
        adopting.status.success(),
        "adopting a running drone is success, not a race the second caller loses"
    );
    assert_eq!(
        drone.state().pid,
        first_pid,
        "the original process is still the one behind the socket"
    );
}

/// `--stop` ends the drone, and the socket and its state file are both gone afterwards.
#[test]
fn stop_ends_the_drone_and_cleans_up_after_it() {
    let mut drone = Drone::listening("never");
    let state_path = drone.state_path();

    let stopped = Command::new(env!("CARGO_BIN_EXE_ubiq-drone"))
        .arg("--stop")
        .arg(&drone.socket)
        .output()
        .expect("running --stop");
    assert!(
        stopped.status.success(),
        "--stop against a live drone succeeds"
    );

    assert!(
        drone.ended_within(PATIENCE),
        "--stop takes the process down"
    );
    assert!(!drone.socket.exists(), "--stop leaves no socket behind it");
    assert!(
        !state_path.exists(),
        "--stop leaves no state file behind it"
    );
}

/// `--list` prunes a state file whose socket is dead: killing the drone out from under its state
/// file leaves a stale record, and `--list` removes it rather than reporting a drone that is gone.
#[test]
fn list_prunes_a_state_file_whose_socket_is_dead() {
    let mut drone = Drone::listening("never");
    let state_path = drone.state_path();
    let _ = drone.state(); // make sure the file exists before the kill races it

    drone.child.kill().expect("killing the drone");
    drone.child.wait().expect("reaping the drone");
    // The socket is not unlinked by a kill -9: this is exactly the crash case the state file's
    // staleness check exists for.
    let deadline = Instant::now() + PATIENCE;
    while UnixStream::connect(&drone.socket).is_ok() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }

    let listed = ubiq_drone::state::list(drone.socket.parent().expect("the socket's directory"));
    assert!(
        listed
            .iter()
            .all(|found| found.socket != drone.socket.to_string_lossy()),
        "a dead drone's state is pruned, not listed"
    );
    assert!(!state_path.exists(), "the pruned state file is removed");
}
