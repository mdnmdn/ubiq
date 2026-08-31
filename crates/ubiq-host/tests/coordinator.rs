//! The coordinator, end to end over the bus: no window, no emulator, just the messages.

use std::io::{Read, Write};
use std::time::Duration;

use ubiq_host::config::{ConfigRoot, RootSource};
use ubiq_host::coordinator;
use ubiq_host::projects::Projects;
use ubiq_host::store::memory::{MemoryPreferenceStore, MemoryProjectStore};
use ubiq_proto::bus::{self, Client, FromClient, Hub};
use ubiq_proto::ids::{PaneId, SessionId};
use ubiq_proto::messages::Message;

/// Long enough for a process to start and say something on a loaded machine.
const PATIENCE: Duration = Duration::from_secs(10);

/// A host with one window attached. The hub comes back with the client because dropping it would
/// close the host's inbox and end the thread under the test.
fn coordinator() -> (Hub, Client) {
    let (hub, host) = bus::hub();
    let root = tempfile::TempDir::new().unwrap();
    let config = ConfigRoot {
        path: root.path().to_path_buf(),
        source: RootSource::Flag,
    };
    let (projects, pending) = Projects::open(
        config.path.clone(),
        Box::new(MemoryProjectStore::new()),
        Box::new(MemoryPreferenceStore::new()),
    );
    // The directory has to outlive the thread that is writing into it.
    std::mem::forget(root);
    coordinator::start(host, config, projects, pending);
    let client = hub.connect();
    (hub, client)
}

/// The pane ID the coordinator answered a spawn with.
fn spawn(ui: &Client, program: &str, args: &[&str]) -> PaneId {
    ui.send(Message::SpawnWorkspace {
        session_id: SessionId::generate(),
        project_id: None,
        agent_type: Some(program.to_string()),
        args: args.iter().map(|a| a.to_string()).collect(),
        folder: None,
    });

    // A window is told what the host is as it attaches, so the answer is not always first.
    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::WorkspaceSpawned { workspace }) => return workspace.id,
            Ok(Message::HostInfo { .. }) => continue,
            other => panic!("expected the workspace, got {other:?}"),
        }
    }
}

/// Collect output until `needle` shows up, or until the pane ends.
fn wait_for_output(ui: &Client, needle: &str) -> String {
    let mut seen = String::new();
    while let Ok(message) = ui.from_host().recv_timeout(PATIENCE) {
        match message {
            Message::TerminalOutput { bytes, .. } => {
                seen.push_str(&String::from_utf8_lossy(&bytes));
                if seen.contains(needle) {
                    return seen;
                }
            }
            Message::PaneExited { .. } => break,
            _ => {}
        }
    }
    panic!("never saw {needle:?}; the pane said {seen:?}");
}

#[test]
fn a_harness_reports_its_output_and_its_exit() {
    let (_hub, ui) = coordinator();
    let pane_id = spawn(&ui, "/bin/echo", &["hello"]);

    let seen = wait_for_output(&ui, "hello");
    assert!(seen.contains("hello"), "output was {seen:?}");

    // The exit follows the output, on the same pane.
    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::PaneExited { pane_id: id, code }) => {
                assert_eq!(id, pane_id);
                assert_eq!(code, 0);
                return;
            }
            Ok(_) => continue,
            Err(_) => panic!("the pane never reported its exit"),
        }
    }
}

#[test]
fn keystrokes_reach_the_harness() {
    let (_hub, ui) = coordinator();
    let pane_id = spawn(&ui, "/bin/cat", &[]);

    ui.send(Message::TerminalInput {
        pane_id,
        bytes: b"ping\n".to_vec(),
    });
    wait_for_output(&ui, "ping");

    // Closing the pane kills the harness, so the reader stops.
    ui.send(Message::CloseWorkspace { pane_id });
}

#[test]
fn a_pane_stream_carries_chunks_and_ends_when_its_sender_goes() {
    let (chunks, mut output) = bus::pane_output();
    chunks.send(b"abcd".to_vec()).unwrap();

    // A chunk larger than the buffer is handed over in pieces, in order.
    let mut buffer = [0u8; 3];
    assert_eq!(output.read(&mut buffer).unwrap(), 3);
    assert_eq!(&buffer, b"abc");
    assert_eq!(output.read(&mut buffer).unwrap(), 1);
    assert_eq!(&buffer[..1], b"d");

    // Dropping the sender is how a pane is told its harness is done.
    drop(chunks);
    assert_eq!(output.read(&mut buffer).unwrap(), 0);
}

#[test]
fn a_keystroke_leaves_as_terminal_input() {
    let (hub, host) = bus::hub();
    let ui = hub.connect();
    let pane_id = PaneId::generate();

    let mut input = ui.input(pane_id);
    input.write_all(b"q").unwrap();

    // The attach comes first; the keystroke is behind it.
    assert!(matches!(host.recv(), Ok(FromClient::Connected(_))));

    match host.recv() {
        Ok(FromClient::Said {
            message: Message::TerminalInput { pane_id: id, bytes },
            ..
        }) => {
            assert_eq!(id, pane_id);
            assert_eq!(bytes, b"q");
        }
        other => panic!("expected the keystroke, got {other:?}"),
    }
}

#[test]
fn a_pane_belongs_to_the_window_that_spawned_it() {
    let (hub, a) = coordinator();
    let b = hub.connect();

    let pane_id = spawn(&a, "/bin/cat", &[]);

    // What b is told on attaching is its own business, and not about this pane.
    while let Ok(message) = b.from_host().recv_timeout(Duration::from_millis(200)) {
        assert!(
            matches!(message, Message::HostInfo { .. }),
            "b should hear nothing about a pane it does not own, got {message:?}"
        );
    }

    // The other window may not drive a pane it does not own, however it learned the id.
    b.send(Message::TerminalInput {
        pane_id,
        bytes: b"from the wrong window\n".to_vec(),
    });
    // …and nothing about that pane is addressed to it.
    assert!(
        b.from_host()
            .recv_timeout(Duration::from_millis(300))
            .is_err(),
        "output for a pane must reach only the window that owns it"
    );

    // The owner still drives it perfectly well.
    a.send(Message::TerminalInput {
        pane_id,
        bytes: b"ping\n".to_vec(),
    });
    wait_for_output(&a, "ping");

    a.send(Message::CloseWorkspace { pane_id });
}

#[test]
fn a_window_that_goes_takes_its_harnesses_with_it() {
    let (hub, a) = coordinator();

    // A harness that keeps writing for far longer than the test will wait. The file is the only
    // way to see it: a pane is an ID and a byte stream, so the coordinator reports no process, and
    // this sandbox does not allow `ps`.
    let beat = std::env::temp_dir().join(format!("ubiq-heartbeat-{}", std::process::id()));
    let _ = std::fs::remove_file(&beat);
    let script = format!(
        "while true; do printf . >> {}; sleep 0.05; done",
        beat.display()
    );
    let pane_id = spawn(&a, "/bin/sh", &["-c", &script]);

    // Wait until it is definitely running.
    let deadline = std::time::Instant::now() + PATIENCE;
    while beats(&beat) == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "the harness never started writing"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // The window closes. Nothing else drops now that the host outlives every window, so the only
    // thing that can reap this harness is the host noticing the client has gone.
    drop(a);

    // Give the kill time to land, then take two readings a long way apart. A harness that survived
    // its window would still be appending between them.
    std::thread::sleep(Duration::from_millis(600));
    let after = beats(&beat);
    std::thread::sleep(Duration::from_millis(600));
    let later = beats(&beat);

    let _ = std::fs::remove_file(&beat);
    drop(hub);

    assert_eq!(
        after,
        later,
        "the harness outlived the window that owned it: it wrote {} more times",
        later - after
    );
    let _ = pane_id;
}

/// How many times the harness has written. Zero if it never started.
fn beats(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}
