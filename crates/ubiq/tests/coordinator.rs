//! The coordinator, end to end over the bus: no window, no emulator, just the messages.

use std::io::{Read, Write};
use std::time::Duration;

use ubiq::bus::{self, UiEnd};
use ubiq::messages::Message;
use ubiq::orchestrator;
use uuid::Uuid;

/// Long enough for a process to start and say something on a loaded machine.
const PATIENCE: Duration = Duration::from_secs(10);

fn coordinator() -> UiEnd {
    let (ui, coordinator) = bus::pair();
    orchestrator::start(coordinator);
    ui
}

/// The pane ID the coordinator answered a spawn with.
fn spawn(ui: &UiEnd, program: &str, args: &[&str]) -> Uuid {
    ui.send(Message::SpawnWorkspace {
        session_id: Uuid::new_v4(),
        agent_type: Some(program.to_string()),
        args: args.iter().map(|a| a.to_string()).collect(),
        folder: None,
    });

    match ui.from_coordinator.recv_timeout(PATIENCE) {
        Ok(Message::WorkspaceSpawned { workspace }) => workspace.id,
        other => panic!("expected the workspace, got {other:?}"),
    }
}

/// Collect output until `needle` shows up, or until the pane ends.
fn wait_for_output(ui: &UiEnd, needle: &str) -> String {
    let mut seen = String::new();
    while let Ok(message) = ui.from_coordinator.recv_timeout(PATIENCE) {
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
    let ui = coordinator();
    let pane_id = spawn(&ui, "/bin/echo", &["hello"]);

    let seen = wait_for_output(&ui, "hello");
    assert!(seen.contains("hello"), "output was {seen:?}");

    // The exit follows the output, on the same pane.
    loop {
        match ui.from_coordinator.recv_timeout(PATIENCE) {
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
    let ui = coordinator();
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
    let (ui, coordinator) = bus::pair();
    let pane_id = Uuid::new_v4();

    let mut input = ui.input(pane_id);
    input.write_all(b"q").unwrap();

    match coordinator.from_ui.recv_timeout(PATIENCE) {
        Ok(Message::TerminalInput { pane_id: id, bytes }) => {
            assert_eq!(id, pane_id);
            assert_eq!(bytes, b"q");
        }
        other => panic!("expected the keystroke, got {other:?}"),
    }
}
