//! The coordinator, end to end over the bus: no window, no emulator, just the messages.

use std::io::{Read, Write};
use std::time::Duration;

use ubiq_host::config::{ConfigRoot, RootSource};
use ubiq_host::coordinator;
use ubiq_host::projects::Projects;
use ubiq_host::store::memory::{MemoryPreferenceStore, MemoryProjectStore};
use ubiq_proto::bus::{self, Client, FromClient, Hub};
use ubiq_proto::files::{FileError, FileVersion};
use ubiq_proto::ids::{PaneId, ProjectId, SessionId};
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

/// Take a folder into the catalogue and answer its id.
///
/// A pane runs in a project's folder, so every spawn needs one — which makes this the path the
/// application itself takes rather than a shortcut around the catalogue.
fn add_project(ui: &Client, path: &std::path::Path) -> ProjectId {
    ui.send(Message::AddProject {
        path: path.to_string_lossy().into_owned(),
        name: None,
        colour: None,
    });

    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::ProjectAdded { project }) => return project.id(),
            Ok(Message::HostInfo { .. }) => continue,
            other => panic!("expected the project, got {other:?}"),
        }
    }
}

/// A project on a temporary folder that outlives the test.
fn a_project(ui: &Client) -> (ProjectId, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.keep();
    (add_project(ui, &path), path)
}

/// The pane ID the coordinator answered a spawn with.
fn spawn(ui: &Client, program: &str, args: &[&str]) -> PaneId {
    let (project_id, _path) = a_project(ui);
    spawn_in(ui, project_id, None, program, args)
}

/// The same, in a project the caller already has.
fn spawn_in(
    ui: &Client,
    project_id: ProjectId,
    rel_path: Option<&str>,
    program: &str,
    args: &[&str],
) -> PaneId {
    ui.send(Message::SpawnWorkspace {
        session_id: SessionId::generate(),
        project_id,
        rel_path: rel_path.map(|p| p.to_string()),
        agent_type: Some(program.to_string()),
        args: args.iter().map(|a| a.to_string()).collect(),
    });

    // A window is told what the host is as it attaches, and a pane opening changes the project's
    // snapshot, so the answer is not always first.
    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::WorkspaceSpawned { workspace }) => {
                assert_eq!(workspace.project_id, project_id);
                return workspace.id;
            }
            Ok(Message::HostInfo { .. } | Message::ProjectChanged { .. }) => continue,
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

    // What b is told on attaching, and about the catalogue every window shares, is its own
    // business — but nothing about this pane.
    while let Ok(message) = b.from_host().recv_timeout(Duration::from_millis(200)) {
        assert!(
            matches!(
                message,
                Message::HostInfo { .. }
                    | Message::ProjectAdded { .. }
                    | Message::ProjectChanged { .. }
            ),
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

// ── a pane runs in its project's folder ─────────────────────────────

#[test]
fn spawning_starts_the_harness_in_the_project_folder() {
    let (_hub, ui) = coordinator();
    let (project_id, path) = a_project(&ui);
    // `pwd` reports the logical path, so the folder is canonicalised the way the host resolved it.
    let canonical = std::fs::canonicalize(&path).unwrap();

    spawn_in(&ui, project_id, None, "/bin/pwd", &[]);
    let seen = wait_for_output(&ui, &canonical.to_string_lossy());
    assert!(
        seen.contains(&*canonical.to_string_lossy()),
        "said {seen:?}"
    );
}

#[test]
fn spawning_with_a_rel_path_starts_below_the_project() {
    let (_hub, ui) = coordinator();
    let (project_id, path) = a_project(&ui);
    std::fs::create_dir(path.join("sub")).unwrap();
    let canonical = std::fs::canonicalize(path.join("sub")).unwrap();

    spawn_in(&ui, project_id, Some("sub"), "/bin/pwd", &[]);
    let seen = wait_for_output(&ui, &canonical.to_string_lossy());
    assert!(
        seen.contains(&*canonical.to_string_lossy()),
        "said {seen:?}"
    );
}

#[test]
fn spawning_in_a_missing_project_is_refused_before_a_pane_exists() {
    let (_hub, ui) = coordinator();
    let (project_id, path) = a_project(&ui);
    std::fs::remove_dir_all(&path).unwrap();

    ui.send(Message::SpawnWorkspace {
        session_id: SessionId::generate(),
        project_id,
        rel_path: None,
        agent_type: Some("/bin/cat".to_string()),
        args: Vec::new(),
    });

    // The refusal names the project, because there is no pane to name — and a pane that was never
    // drawn leaves nothing on screen to close.
    let mut marked = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut refused = false;
    while std::time::Instant::now() < deadline {
        match ui.from_host().recv_timeout(Duration::from_millis(300)) {
            Ok(Message::ProjectError { project_id: id, .. }) => {
                assert_eq!(id, Some(project_id));
                refused = true;
            }
            // The refusal re-probes, so every picker marks the row from the probe that just
            // happened.
            Ok(Message::ProjectChanged { project }) if project.id() == project_id => {
                marked = !project.health.is_ok();
            }
            Ok(Message::WorkspaceSpawned { .. }) => panic!("a pane was started anyway"),
            Ok(Message::PaneError { .. }) => panic!("a pane that was never drawn cannot error"),
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(refused, "the spawn was not refused");
    assert!(marked, "the project's row was not re-probed");
}

#[test]
fn spawning_with_a_rel_path_that_escapes_is_refused() {
    let (_hub, ui) = coordinator();
    let (project_id, _path) = a_project(&ui);

    ui.send(Message::SpawnWorkspace {
        session_id: SessionId::generate(),
        project_id,
        rel_path: Some("../..".to_string()),
        agent_type: Some("/bin/cat".to_string()),
        args: Vec::new(),
    });

    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::ProjectError { project_id: id, .. }) => {
                assert_eq!(id, Some(project_id));
                return;
            }
            Ok(Message::WorkspaceSpawned { .. }) => panic!("a pane started outside its project"),
            Ok(_) => continue,
            Err(_) => panic!("the escape was neither refused nor honoured"),
        }
    }
}

// ── the file family over the bus ────────────────────────────────────

#[test]
fn a_tree_request_is_answered_to_the_window_that_asked() {
    let (hub, ui) = coordinator();
    let other = hub.connect();
    let (project_id, path) = a_project(&ui);
    std::fs::write(
        path.join("README.md"),
        b"hello
",
    )
    .unwrap();
    std::fs::create_dir(path.join("crates")).unwrap();

    ui.send(Message::ProjectTree {
        project_id,
        rel_path: String::new(),
        depth: 1,
    });

    let listings = loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::ProjectTreeListing {
                project_id: id,
                rel_path,
                listings,
            }) => {
                assert_eq!(id, project_id);
                assert_eq!(rel_path, "");
                break listings;
            }
            Ok(_) => continue,
            Err(_) => panic!("the tree was never answered"),
        }
    };

    let names: Vec<&str> = listings[0]
        .entries
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(names, vec!["crates", "README.md"]);

    // A listing goes to the window that asked, never to every window.
    while let Ok(message) = other.from_host().recv_timeout(Duration::from_millis(200)) {
        assert!(
            !matches!(message, Message::ProjectTreeListing { .. }),
            "a listing reached a window that did not ask for it"
        );
    }
}

#[test]
fn a_file_request_for_an_unknown_project_is_refused() {
    let (_hub, ui) = coordinator();
    let project_id = ProjectId::generate();

    ui.send(Message::ReadProjectFile {
        project_id,
        rel_path: "anything".to_string(),
        max_bytes: None,
    });

    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::ProjectFileError { error, .. }) => {
                assert!(matches!(error, FileError::Refused(_)), "answered {error:?}");
                return;
            }
            Ok(_) => continue,
            Err(_) => panic!("an unknown project was never refused"),
        }
    }
}

#[test]
fn a_read_and_a_save_round_trip_over_the_bus() {
    let (_hub, ui) = coordinator();
    let (project_id, path) = a_project(&ui);
    std::fs::write(
        path.join("notes.txt").as_path(),
        b"before
",
    )
    .unwrap();

    ui.send(Message::ReadProjectFile {
        project_id,
        rel_path: "notes.txt".to_string(),
        max_bytes: None,
    });
    let read = expect_contents(&ui);
    assert_eq!(
        read.bytes,
        b"before
"
    );
    let version = read.version.expect("a whole read carries a version");

    ui.send(Message::WriteProjectFile {
        project_id,
        rel_path: "notes.txt".to_string(),
        bytes: b"after
"
        .to_vec(),
        expected: Some(version),
    });
    let written = expect_written(&ui);
    assert_ne!(written, version, "the version has to move with the file");
    assert_eq!(
        std::fs::read(path.join("notes.txt")).unwrap(),
        b"after
"
    );

    // The version the interface still holds is now stale, and a second save on it is refused
    // rather than clobbering what is there.
    ui.send(Message::WriteProjectFile {
        project_id,
        rel_path: "notes.txt".to_string(),
        bytes: b"clobbered
"
        .to_vec(),
        expected: Some(version),
    });
    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::ProjectFileError { error, .. }) => {
                assert_eq!(error, FileError::Conflict);
                break;
            }
            Ok(Message::ProjectFileWritten { .. }) => panic!("a stale save was allowed"),
            Ok(_) => continue,
            Err(_) => panic!("the stale save was never answered"),
        }
    }
    assert_eq!(
        std::fs::read(path.join("notes.txt")).unwrap(),
        b"after
"
    );
}

fn expect_contents(ui: &Client) -> ubiq_proto::files::FileContents {
    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::ProjectFileContents { contents, .. }) => return contents,
            Ok(Message::ProjectFileError { error, .. }) => panic!("the read failed: {error:?}"),
            Ok(_) => continue,
            Err(_) => panic!("the read was never answered"),
        }
    }
}

fn expect_written(ui: &Client) -> FileVersion {
    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::ProjectFileWritten { version, .. }) => return version,
            Ok(Message::ProjectFileError { error, .. }) => panic!("the save failed: {error:?}"),
            Ok(_) => continue,
            Err(_) => panic!("the save was never answered"),
        }
    }
}
