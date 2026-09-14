//! A drone that outlives its link, driven against the real binary.
//!
//! Everything here runs the actual executable with `--listen`, talks to it over the actual unix
//! socket, and asserts against the process: that it is still there when its client is not, that
//! the pane it holds is the *same* pane when somebody attaches again, that the screen comes back
//! with it, and that `--linger 0` means what it says. A test that drove `Relay` in-process would
//! prove none of those — the thing under test is a lifecycle, and a lifecycle only exists in a
//! process.
//!
//! Unix only, as the feature is: the socket, its mode bits and the multiplexer that holds the
//! process have no Windows spelling that would mean the same thing.
#![cfg(unix)]

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ubiq_drone::scrollback::RING_BYTES;
use ubiq_proto::carrier::welcome;
use ubiq_proto::ids::{PaneId, SessionId};
use ubiq_proto::messages::Message;
use ubiq_proto::wire;

/// How long a test waits for one expected message. Generous: a cold process on a loaded machine is
/// slow, and a flaky test is worse than a slow one.
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
    /// the test holds the process itself, because what is under test is the socket and never the
    /// thing holding the process.
    fn listening(linger: &str) -> Self {
        let home = tempfile::tempdir().expect("a home for this test");
        let project = home.path().join("project");
        std::fs::create_dir_all(&project).expect("the project folder");
        let socket = home.path().join("drone.sock");

        let child = Command::new(env!("CARGO_BIN_EXE_ubiq-drone"))
            .arg("--listen")
            .arg(&socket)
            .arg("--foreground")
            .args(["--linger", linger])
            .arg("--root")
            .arg(&project)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("starting a drone");

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

    fn attach(&self) -> Link {
        Link::open(&self.socket)
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

/// One attach: the preamble, the handshake, and raw frames after it.
///
/// Raw frames rather than a `bus::Client`, following `handshake.rs`: what these tests are about is
/// which frames crossed and in which order, which a client would hide.
struct Link {
    reader: UnixStream,
    writer: UnixStream,
}

impl Link {
    fn open(socket: &Path) -> Self {
        let mut writer = UnixStream::connect(socket).expect("dialling the drone");
        writer
            .set_read_timeout(Some(PATIENCE))
            .expect("a read timeout");
        ubiq_drone::socket::write_preamble(&mut writer, None).expect("the attach preamble");
        let mut reader = writer.try_clone().expect("the read half");
        reader
            .set_read_timeout(Some(PATIENCE))
            .expect("a read timeout");
        welcome(&mut reader, &mut writer, "ubiq-drone-tests").expect("the handshake");
        Self { reader, writer }
    }

    fn say(&mut self, message: Message) {
        wire::write_frame(&mut self.writer, &message).expect("writing a frame");
        self.writer.flush().expect("flushing");
    }

    /// The first message matching `want`, or a panic naming what did arrive.
    fn wait_for<T>(&mut self, what: &str, mut want: impl FnMut(&Message) -> Option<T>) -> T {
        let deadline = Instant::now() + PATIENCE;
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            let Ok(message) = wire::read_frame(&mut self.reader) else {
                break;
            };
            if let Some(found) = want(&message) {
                return found;
            }
            seen.push(format!("{message:?}").chars().take(120).collect::<String>());
        }
        panic!("never saw {what}; what arrived was {seen:#?}");
    }

    /// The project this drone was launched with.
    fn project(&mut self) -> ubiq_proto::ids::ProjectId {
        self.say(Message::ListProjects);
        self.wait_for("ProjectList", |message| match message {
            Message::ProjectList { projects } => Some(projects[0].id()),
            _ => None,
        })
    }

    /// A shell, and the pane it runs in.
    fn open_pane(&mut self) -> PaneId {
        let project_id = self.project();
        self.say(Message::SpawnWorkspace {
            session_id: SessionId::generate(),
            project_id,
            rel_path: None,
            agent_type: Some("/bin/sh".to_string()),
            args: Vec::new(),
            picks: Default::default(),
        });
        self.wait_for("WorkspaceSpawned", |message| match message {
            Message::WorkspaceSpawned { workspace } => Some(workspace.id),
            _ => None,
        })
    }

    /// Read until this pane's output has said `text`, and answer with everything read.
    fn until_said(&mut self, pane_id: PaneId, text: &str) -> Vec<u8> {
        let mut screen = Vec::new();
        self.wait_for(text, |message| match message {
            Message::TerminalOutput {
                pane_id: who,
                bytes,
            } if *who == pane_id => {
                screen.extend_from_slice(bytes);
                String::from_utf8_lossy(&screen)
                    .contains(text)
                    .then_some(())
            }
            _ => None,
        });
        screen
    }

    /// Drop the link the way a dead `ssh` does: both directions at once, with nothing said.
    fn drop_the_link(self) {
        let _ = self.writer.shutdown(std::net::Shutdown::Both);
    }
}

/// The whole of phase 6 in one run: the client goes, the drone stays, the pane keeps running, and
/// attaching again finds the *same* pane with its screen still on it.
#[test]
fn a_detached_drone_holds_its_panes_and_replays_them() {
    let drone = Drone::listening("60");
    let mut first = drone.attach();
    first.wait_for("HostInfo", |message| {
        matches!(message, Message::HostInfo { .. }).then_some(())
    });

    let pane_id = first.open_pane();
    first.say(Message::TerminalInput {
        pane_id,
        bytes: b"echo held-across-links\n".to_vec(),
    });
    first.until_said(pane_id, "held-across-links");

    // The link dies with nothing said about it — the case the whole feature exists for.
    first.drop_the_link();
    std::thread::sleep(Duration::from_millis(500));

    // Proof the pane is still alive, not merely remembered: it writes a file from inside itself
    // after the reattach.
    let mut again = drone.attach();
    again.wait_for("HostInfo", |message| {
        matches!(message, Message::HostInfo { .. }).then_some(())
    });
    let announced = again.wait_for("the pane being re-announced", |message| match message {
        Message::WorkspaceSpawned { workspace } => Some(workspace.id),
        _ => None,
    });
    assert_eq!(
        announced, pane_id,
        "a pane id is minted by the drone and stays stable across links"
    );

    let replayed = again.until_said(pane_id, "held-across-links");
    assert!(
        String::from_utf8_lossy(&replayed).contains("held-across-links"),
        "the ring is what stops a reattached pane being live but blank"
    );

    let marker = drone.project.join("still-alive");
    again.say(Message::TerminalInput {
        pane_id,
        bytes: format!("touch {}\n", marker.display()).into_bytes(),
    });
    let deadline = Instant::now() + PATIENCE;
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        marker.exists(),
        "the held pane is a live process, not a remembered one"
    );
}

/// The ring is bounded, and what it drops is the oldest. Asserted against the process: a pane is
/// made to produce far more than the ring holds, and what comes back on the reattach is the end of
/// it and no more than the bound.
#[test]
fn the_replayed_ring_is_bounded_and_keeps_the_newest() {
    let drone = Drone::listening("60");
    let mut first = drone.attach();
    first.wait_for("HostInfo", |message| {
        matches!(message, Message::HostInfo { .. }).then_some(())
    });
    let pane_id = first.open_pane();

    // Comfortably more than 256 KiB, then a marker nothing can push out but more output.
    first.say(Message::TerminalInput {
        pane_id,
        bytes: b"i=0; while [ $i -lt 4000 ]; do echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa; i=$((i+1)); done; echo the-newest-line\n".to_vec(),
    });
    first.until_said(pane_id, "the-newest-line");
    first.drop_the_link();
    std::thread::sleep(Duration::from_millis(500));

    let mut again = drone.attach();
    again.wait_for("the pane being re-announced", |message| match message {
        Message::WorkspaceSpawned { workspace } if workspace.id == pane_id => Some(()),
        _ => None,
    });
    let replayed = again.wait_for("the replayed screen", |message| match message {
        Message::TerminalOutput {
            pane_id: who,
            bytes,
        } if *who == pane_id => Some(bytes.clone()),
        _ => None,
    });
    assert!(
        replayed.len() <= RING_BYTES,
        "the ring held {} bytes, over its {RING_BYTES} bound",
        replayed.len()
    );
    assert!(
        String::from_utf8_lossy(&replayed).contains("the-newest-line"),
        "the oldest bytes are the ones that go, not the newest"
    );
}

/// `--linger 0` is the attached drone's lifetime with a socket in front of it: the last client to
/// leave takes the process with it.
#[test]
fn linger_zero_exits_when_the_last_client_leaves() {
    let mut drone = Drone::listening("0");
    let mut link = drone.attach();
    link.wait_for("HostInfo", |message| {
        matches!(message, Message::HostInfo { .. }).then_some(())
    });
    let pane_id = link.open_pane();
    link.say(Message::TerminalInput {
        pane_id,
        bytes: b"echo the-pane-is-up\n".to_vec(),
    });
    link.until_said(pane_id, "the-pane-is-up");

    assert!(
        !drone.ended_within(Duration::from_millis(500)),
        "a drone with a client attached does not go anywhere"
    );

    link.drop_the_link();
    assert!(
        drone.ended_within(PATIENCE),
        "--linger 0 means the last client to leave takes the drone with it"
    );
    assert!(
        !drone.socket.exists(),
        "a drone that has gone leaves no socket for the next one to trip over"
    );
}

/// A linger re-asserted on an attach is the one that counts: a drone started to wait ten minutes
/// is told `0` by the client that attaches to it, and goes when that client does — no restart, and
/// no pane lost to one.
#[test]
fn an_attach_re_asserts_the_linger() {
    let mut drone = Drone::listening("never");
    let mut link = Link::open(&drone.socket);
    link.wait_for("HostInfo", |message| {
        matches!(message, Message::HostInfo { .. }).then_some(())
    });
    drop(link);

    // A second attach, this one naming a linger of its own.
    let mut writer = UnixStream::connect(&drone.socket).expect("dialling the drone");
    writer
        .set_read_timeout(Some(PATIENCE))
        .expect("a read timeout");
    ubiq_drone::socket::write_preamble(&mut writer, Some(ubiq_drone::linger::Linger::Immediate))
        .expect("the attach preamble");
    let mut reader = writer.try_clone().expect("the read half");
    welcome(&mut reader, &mut writer, "ubiq-drone-tests").expect("the handshake");
    let _ = wire::read_frame(&mut reader).expect("HostInfo");

    let _ = writer.shutdown(std::net::Shutdown::Both);
    assert!(
        drone.ended_within(PATIENCE),
        "the linger an attach asserts is the one the drone lives by"
    );
}
