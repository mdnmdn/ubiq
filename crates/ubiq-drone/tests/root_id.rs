//! `--root-id`, driven against the real binary.
//!
//! A drone launched with an id bound to its root announces that exact id in its `ListProjects`
//! answer, rather than minting a fresh one — which is what lets the interface see one project
//! across a launch and a reattach, not two. Against the real process, not `Relay` in-process,
//! because argv parsing is what is under test.

#![cfg(unix)]

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ubiq_proto::carrier::welcome;
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;
use ubiq_proto::wire;

/// How long a test waits for one expected message. Generous: a cold process on a loaded machine is
/// slow, and a flaky test is worse than a slow one.
const PATIENCE: Duration = Duration::from_secs(20);

/// A drone process listening on a socket of this test's own, so an attach can be driven over raw
/// frames the same way `detach.rs` and `handshake.rs` do.
struct Drone {
    child: Child,
    _home: tempfile::TempDir,
}

impl Drop for Drone {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drone {
    fn listening(socket: &std::path::Path, project: &std::path::Path, id: ProjectId) -> Self {
        let home = tempfile::tempdir().expect("a home for this test");
        let child = Command::new(env!("CARGO_BIN_EXE_ubiq-drone"))
            .arg("--listen")
            .arg(socket)
            .arg("--foreground")
            .arg("--root")
            .arg(project)
            .arg("--root-id")
            .arg(id.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("starting a drone");

        let deadline = Instant::now() + PATIENCE;
        while Instant::now() < deadline && UnixStream::connect(socket).is_err() {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(
            UnixStream::connect(socket).is_ok(),
            "the drone never bound its socket"
        );

        Self { child, _home: home }
    }
}

#[test]
fn a_root_id_is_announced_verbatim() {
    let home = tempfile::tempdir().expect("a home for this test");
    let project = home.path().join("project");
    std::fs::create_dir_all(&project).expect("the project folder");
    let socket = home.path().join("drone.sock");
    let id = ProjectId::generate();

    let _drone = Drone::listening(&socket, &project, id);

    let mut writer = UnixStream::connect(&socket).expect("dialling the drone");
    writer
        .set_read_timeout(Some(PATIENCE))
        .expect("a read timeout");
    ubiq_drone::socket::write_preamble(&mut writer, None).expect("the attach preamble");
    let mut reader = writer.try_clone().expect("the read half");
    reader
        .set_read_timeout(Some(PATIENCE))
        .expect("a read timeout");
    welcome(&mut reader, &mut writer, "ubiq-drone-tests").expect("the handshake");

    // The first frame is `HostInfo`, unasked, exactly as `Coordinator::client_here` sends it; the
    // answer this test is about is the one after it.
    match wire::read_frame(&mut reader).expect("a frame from the drone") {
        Message::HostInfo { .. } => {}
        other => panic!("expected HostInfo first, got {other:?}"),
    }

    wire::write_frame(&mut writer, &Message::ListProjects).expect("asking for the catalogue");
    writer.flush().expect("flushing");
    let announced = match wire::read_frame(&mut reader).expect("a frame from the drone") {
        Message::ProjectList { projects } => {
            assert_eq!(projects.len(), 1, "one root, one project");
            projects[0].id()
        }
        other => panic!("expected ProjectList, got {other:?}"),
    };

    assert_eq!(
        announced, id,
        "a root bound to an id must announce that exact id, not a fresh one"
    );
}
