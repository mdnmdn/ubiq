//! `ServeProject`, driven against the real binary started with **no** `--root`.
//!
//! A drone serves whatever a window asks it to, under the id the window's own catalogue holds —
//! which is what lets a project pinned to a machine work whatever the drone there was launched
//! with. Against the real process, so the rootless argv is under test too.

#![cfg(unix)]

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ubiq_proto::carrier::welcome;
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;
use ubiq_proto::wire;

/// How long a test waits for one expected message.
const PATIENCE: Duration = Duration::from_secs(20);

struct Drone(Child);

impl Drop for Drone {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A drone listening on `socket`, launched with no root at all.
fn rootless(socket: &std::path::Path) -> Drone {
    let child = Command::new(env!("CARGO_BIN_EXE_ubiq-drone"))
        .arg("--listen")
        .arg(socket)
        .arg("--foreground")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("starting a drone");
    let drone = Drone(child);
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline && UnixStream::connect(socket).is_err() {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        UnixStream::connect(socket).is_ok(),
        "a drone with no --root never bound its socket"
    );
    drone
}

/// Attach and handshake; the stream is past the unasked `HostInfo`.
fn attach(socket: &std::path::Path) -> (UnixStream, UnixStream) {
    let mut writer = UnixStream::connect(socket).expect("dialling the drone");
    writer.set_read_timeout(Some(PATIENCE)).expect("a timeout");
    ubiq_drone::socket::write_preamble(&mut writer, None).expect("the attach preamble");
    let mut reader = writer.try_clone().expect("the read half");
    reader.set_read_timeout(Some(PATIENCE)).expect("a timeout");
    welcome(&mut reader, &mut writer, "ubiq-drone-tests").expect("the handshake");
    match wire::read_frame(&mut reader).expect("a frame") {
        Message::HostInfo { .. } => {}
        other => panic!("expected HostInfo first, got {other:?}"),
    }
    (reader, writer)
}

fn say(writer: &mut UnixStream, message: &Message) {
    wire::write_frame(writer, message).expect("a frame out");
    writer.flush().expect("flushing");
}

/// The next frame this test is about, skipping the drone's unasked chatter.
fn next(reader: &mut UnixStream, want: impl Fn(&Message) -> bool) -> Message {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        let message = wire::read_frame(reader).expect("a frame from the drone");
        if want(&message) {
            return message;
        }
    }
    panic!("the expected frame never came");
}

#[test]
fn a_rootless_drone_starts_empty_and_serves_a_folder_it_is_asked_to() {
    let home = tempfile::tempdir().expect("a home for this test");
    let project = home.path().join("project");
    std::fs::create_dir_all(&project).expect("the project folder");
    std::fs::write(project.join("hello.txt"), "hi").expect("a file in it");
    let socket = home.path().join("drone.sock");
    let _drone = rootless(&socket);
    let (mut reader, mut writer) = attach(&socket);

    say(&mut writer, &Message::ListProjects);
    match next(&mut reader, |m| matches!(m, Message::ProjectList { .. })) {
        Message::ProjectList { projects } => assert!(projects.is_empty(), "no root, no project"),
        _ => unreachable!(),
    }

    let id = ProjectId::generate();
    let serve = Message::ServeProject {
        project_id: id,
        path: project.to_string_lossy().into_owned(),
    };
    say(&mut writer, &serve);
    match next(&mut reader, |m| matches!(m, Message::ProjectList { .. })) {
        Message::ProjectList { projects } => {
            assert_eq!(projects.len(), 1);
            assert_eq!(projects[0].id(), id, "served under the asker's own id");
        }
        _ => unreachable!(),
    }
    // Idempotent: asked again, still one project.
    say(&mut writer, &serve);
    match next(&mut reader, |m| matches!(m, Message::ProjectList { .. })) {
        Message::ProjectList { projects } => assert_eq!(projects.len(), 1),
        _ => unreachable!(),
    }

    say(
        &mut writer,
        &Message::ProjectTree {
            project_id: id,
            rel_path: String::new(),
            depth: 1,
            prefetch: false,
        },
    );
    match next(&mut reader, |m| {
        matches!(
            m,
            Message::ProjectTreeListing { .. } | Message::ProjectFileError { .. }
        )
    }) {
        Message::ProjectTreeListing {
            project_id,
            listings,
            ..
        } => {
            assert_eq!(project_id, id);
            let names = format!("{listings:?}");
            assert!(names.contains("hello.txt"), "the folder's file: {names}");
        }
        other => panic!("expected the tree, got {other:?}"),
    }
}

#[test]
fn a_folder_that_is_not_there_is_refused() {
    let home = tempfile::tempdir().expect("a home for this test");
    let socket = home.path().join("drone.sock");
    let _drone = rootless(&socket);
    let (mut reader, mut writer) = attach(&socket);

    let id = ProjectId::generate();
    say(
        &mut writer,
        &Message::ServeProject {
            project_id: id,
            path: home.path().join("gone").to_string_lossy().into_owned(),
        },
    );
    match next(&mut reader, |m| matches!(m, Message::ProjectError { .. })) {
        Message::ProjectError { project_id, .. } => assert_eq!(project_id, Some(id)),
        _ => unreachable!(),
    }
}
