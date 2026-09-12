//! One real session, end to end: a client on one side of a duplex stream, the relay on the other,
//! and nothing in between but the pump and the wire.
//!
//! It drives the whole binary's shape — the stream, the framing, the hub, the relay and a live
//! pseudo-terminal — because every interesting failure in a drone is at a seam. A loopback TCP
//! pair stands in for the `ssh` channel: two independent halves per side, the same shape standard
//! input and output have, and portable in a way a unix socket pair is not.
//!
//! The families a drone refuses and that have **no error variant to refuse with** are not covered
//! here because there is nothing to assert: `ListStats`, `ListNotifications` and the rest of the
//! notification family, `CliShortcut`, and every host→UI answer that arrives at the wrong end of
//! the wire. Those are logged and dropped — see the catch-all in `relay::dispatch`.

use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ubiq_drone::relay::Relay;
use ubiq_host::carrier::{self, NoCloser};
use ubiq_proto::bus;
use ubiq_proto::ids::SessionId;
use ubiq_proto::messages::Message;
use ubiq_proto::work::AgentId;

/// How long a test waits for one expected message. Generous: a cold pseudo-terminal on a loaded
/// machine is slow, and a flaky test is worse than a slow one.
const PATIENCE: Duration = Duration::from_secs(20);

/// A client attached to a relay over a real duplex stream.
struct Session {
    client: bus::Client,
    /// The client's own end of the stream, kept only so dropping the session can shut it down.
    /// Dropping the handles is not enough — the reader thread still holds a clone and would stay
    /// blocked, so the relay would never see the end of the stream. Which is exactly the mistake
    /// `carrier::Closer` exists to name.
    far: TcpStream,
    _relay: std::thread::JoinHandle<()>,
    _pump: std::thread::JoinHandle<()>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.far.shutdown(std::net::Shutdown::Both);
    }
}

impl Session {
    fn open(roots: Vec<PathBuf>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let address = listener.local_addr().expect("the bound address");
        let far = TcpStream::connect(address).expect("dialling the relay");
        let (near, _) = listener.accept().expect("accepting the client");

        // The relay's side: the hub the pump attaches a client to, and the run loop behind it.
        let (hub, host) = bus::hub();
        let relay = std::thread::spawn(move || Relay::new(roots).run(host));
        let pump = std::thread::spawn(move || {
            let reader = near.try_clone().expect("the read half");
            carrier::pump(Box::new(reader), Box::new(near), Box::new(NoCloser), hub);
        });

        // The client's side: a detached client, with two threads doing what the interface's own
        // router does — encode what it says, decode what arrives.
        let (client, detached) = bus::detached();
        let said = detached.said().clone();
        let deliver = detached.deliver().clone();
        let mut out = far.try_clone().expect("the client write half");
        std::thread::spawn(move || {
            while let Ok(event) = said.recv() {
                if let bus::FromClient::Said { message, .. } = event
                    && ubiq_proto::wire::write_frame(&mut out, &message).is_err()
                {
                    break;
                }
            }
        });
        let mut incoming = far.try_clone().expect("the client read half");
        std::thread::spawn(move || {
            while let Ok(message) = ubiq_proto::wire::read_frame(&mut incoming) {
                if deliver.send(message).is_err() {
                    break;
                }
            }
        });

        Self {
            client,
            far,
            _relay: relay,
            _pump: pump,
        }
    }

    fn say(&self, message: Message) {
        self.client.send(message);
    }

    /// The first message matching `want`, or a panic naming what did arrive.
    fn wait_for<T>(&self, what: &str, mut want: impl FnMut(&Message) -> Option<T>) -> T {
        let deadline = Instant::now() + PATIENCE;
        let mut seen = Vec::new();
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.client.from_host().recv_timeout(left) {
                Ok(message) => {
                    if let Some(found) = want(&message) {
                        return found;
                    }
                    seen.push(format!("{message:?}").chars().take(120).collect::<String>());
                }
                Err(_) => break,
            }
        }
        panic!("never saw {what}; what arrived was {seen:#?}");
    }
}

/// The whole of a drone's job in one run: it says what machine it is, lists what it was launched
/// with, serves a live terminal both ways, resizes it, and closes it.
#[test]
fn a_session_serves_a_terminal_and_the_files_beside_it() {
    let project = tempfile::tempdir().expect("a project folder");
    std::fs::write(project.path().join("hello.txt"), b"from the far machine")
        .expect("seeding a file");
    let session = Session::open(vec![project.path().to_path_buf()]);

    // Attaching is answered without being asked: the interface cannot read the far machine's disk.
    let hostname = session.wait_for("HostInfo", |message| match message {
        Message::HostInfo { triplet, .. } => Some(triplet.clone()),
        _ => None,
    });
    assert!(hostname.is_some(), "a drone states its own triplet");

    // The catalogue is what `--root` seeded, and nothing else.
    session.say(Message::ListProjects);
    let project_id = session.wait_for("ProjectList", |message| match message {
        Message::ProjectList { projects } => {
            assert_eq!(projects.len(), 1, "one root, one project");
            Some(projects[0].id())
        }
        _ => None,
    });

    // The file family, answered by the worker on the far machine.
    session.say(Message::ReadProjectFile {
        project_id,
        rel_path: "hello.txt".to_string(),
        max_bytes: None,
    });
    session.wait_for("ProjectFileContents", |message| match message {
        Message::ProjectFileContents { rel_path, .. } if rel_path == "hello.txt" => Some(()),
        _ => None,
    });

    // A pane. `agent_type` is a program name here and nothing more — a drone composes nothing.
    session.say(Message::SpawnWorkspace {
        session_id: SessionId::generate(),
        project_id,
        rel_path: None,
        agent_type: Some(shell()),
        args: Vec::new(),
        picks: Default::default(),
    });
    let pane_id = session.wait_for("WorkspaceSpawned", |message| match message {
        Message::WorkspaceSpawned { workspace } => Some(workspace.id),
        _ => None,
    });

    // Bytes in, bytes out, opaque at both ends.
    session.say(Message::TerminalInput {
        pane_id,
        bytes: b"echo drone-is-alive\n".to_vec(),
    });
    let mut screen = Vec::new();
    session.wait_for("the echo's output", |message| match message {
        Message::TerminalOutput { bytes, .. } => {
            screen.extend_from_slice(bytes);
            // The command echoes back as it is typed, so the answer is the *second* occurrence.
            String::from_utf8_lossy(&screen)
                .matches("drone-is-alive")
                .nth(1)
                .map(|_| ())
        }
        _ => None,
    });

    // A resize that reaches the pseudo-terminal is one the harness is told about; a refused one
    // would come back as a `PaneError`, which is what the assertion below would catch.
    session.say(Message::TerminalResize {
        pane_id,
        cols: 132,
        rows: 43,
    });
    session.say(Message::TerminalInput {
        pane_id,
        bytes: b"stty size\n".to_vec(),
    });
    session.wait_for("the new width", |message| match message {
        Message::TerminalOutput { bytes, .. } => {
            screen.extend_from_slice(bytes);
            String::from_utf8_lossy(&screen)
                .contains("132")
                .then_some(())
        }
        Message::PaneError { error, .. } => panic!("the resize was refused: {error}"),
        _ => None,
    });

    // A refused family answers, and the answer is typed. Silence is the failure this asserts
    // against: an interface waiting on a reply it will never get.
    let agent_id = AgentId::generate();
    session.say(Message::PromptAgent {
        agent_id,
        text: "anything".to_string(),
    });
    session.wait_for("ConversationError", |message| match message {
        Message::ConversationError { agent_id: who, .. } if *who == agent_id => Some(()),
        _ => None,
    });

    // Closing takes the pseudo-terminal with it.
    session.say(Message::CloseWorkspace { pane_id });
    session.wait_for("PaneExited", |message| match message {
        Message::PaneExited { pane_id: gone, .. } if *gone == pane_id => Some(()),
        _ => None,
    });
}

/// A shell that is definitely there, told to stay quiet about it. `sh` on Unix; on Windows the
/// default program is what the host itself would pick.
fn shell() -> String {
    if cfg!(windows) {
        ubiq_host::shells::default_program()
    } else {
        "/bin/sh".to_string()
    }
}

/// A drone never leaves a pseudo-terminal behind on somebody else's machine: the end of the stream
/// is the end of every pane it opened.
#[test]
fn the_end_of_the_stream_kills_every_pane() {
    let project = tempfile::tempdir().expect("a project folder");
    let session = Session::open(vec![project.path().to_path_buf()]);
    session.wait_for("HostInfo", |message| {
        matches!(message, Message::HostInfo { .. }).then_some(())
    });
    session.say(Message::ListProjects);
    let project_id = session.wait_for("ProjectList", |message| match message {
        Message::ProjectList { projects } => Some(projects[0].id()),
        _ => None,
    });

    session.say(Message::SpawnWorkspace {
        session_id: SessionId::generate(),
        project_id,
        rel_path: None,
        agent_type: Some(shell()),
        args: Vec::new(),
        picks: Default::default(),
    });
    let pane_id = session.wait_for("WorkspaceSpawned", |message| match message {
        Message::WorkspaceSpawned { workspace } => Some(workspace.id),
        _ => None,
    });

    // Write a marker file from inside the pane, so the assertion is about the *process* being
    // gone rather than about a message the relay chose to send.
    // A foreground loop, deliberately: a backgrounded one would outlive the shell that was killed
    // and the test would be asserting about the wrong process.
    let marker = project.path().join("alive");
    session.say(Message::TerminalInput {
        pane_id,
        bytes: format!(
            "while true; do touch {}; sleep 0.1; done\n",
            marker.display()
        )
        .into_bytes(),
    });
    let deadline = Instant::now() + PATIENCE;
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(marker.exists(), "the pane never started touching the file");

    // The stream ends. Dropping the client is what the far end sees as EOF.
    drop(session);
    std::thread::sleep(Duration::from_millis(500));
    std::fs::remove_file(&marker).expect("clearing the marker");
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        !marker.exists(),
        "a pane outlived the session that owned it"
    );
}
