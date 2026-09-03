//! The coordinator, end to end over the bus: no window, no emulator, just the messages.

use std::io::{Read, Write};
use std::time::Duration;

use ubiq_host::config::{ConfigRoot, RootSource};
use ubiq_host::coordinator;
use ubiq_host::projects::Projects;
use ubiq_host::settings::Settings;
use ubiq_host::store::memory::{
    MemoryPreferenceStore, MemoryProjectStore, MemorySettingsStore, MemoryTaskStore,
};
use ubiq_host::work::Work;
use ubiq_proto::bus::{self, Client, FromClient, Hub};
use ubiq_proto::conversation::ConvUpdate;
use ubiq_proto::files::{DiffBase, DiffRowKind, FileError, FileVersion};
use ubiq_proto::ids::{PaneId, ProjectId, SessionId};
use ubiq_proto::messages::Message;
use ubiq_proto::settings::{HostSettings, SettingsLayer};
use ubiq_proto::work::AgentId;

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
    let work = Work::open(Box::new(MemoryTaskStore::new()));
    let settings = Settings::open(Box::new(MemorySettingsStore::new()));
    coordinator::start(host, config, projects, work, settings, pending);
    let client = hub.connect();
    (hub, client)
}

/// The same, with the real stores under a directory the test can read back.
///
/// Every other test here runs on the memory stores, which is right: what they assert is what the
/// coordinator says, not what it wrote. This one exists because nothing else proves the two halves
/// meet — that a `ListWork` over the bus ends in bytes on disk, in the place the layout says, in a
/// format that reads back.
fn coordinator_on_disk() -> (Hub, Client, std::path::PathBuf) {
    let (hub, host) = bus::hub();
    let root = tempfile::TempDir::new().unwrap();
    let path = root.path().to_path_buf();
    let config = ConfigRoot {
        path: path.clone(),
        source: RootSource::Flag,
    };
    let (projects, pending) = Projects::open(
        config.path.clone(),
        Box::new(MemoryProjectStore::new()),
        Box::new(MemoryPreferenceStore::new()),
    );
    std::mem::forget(root);
    let work = Work::open(Box::new(ubiq_host::store::file::FileTaskStore::new(
        path.clone(),
    )));
    let settings = Settings::open(Box::new(MemorySettingsStore::new()));
    coordinator::start(host, config, projects, work, settings, pending);
    let client = hub.connect();
    (hub, client, path)
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
fn a_git_request_is_answered_to_the_window_that_asked() {
    let (hub, ui) = coordinator();
    let other = hub.connect();
    let (project_id, path) = a_project(&ui);
    let output = std::process::Command::new("git")
        .current_dir(&path)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Ubiq")
        .env("GIT_AUTHOR_EMAIL", "ubiq@example.invalid")
        .env("GIT_COMMITTER_NAME", "Ubiq")
        .env("GIT_COMMITTER_EMAIL", "ubiq@example.invalid")
        .args(["init", "-q", "-b", "main"])
        .output()
        .expect("git");
    assert!(output.status.success(), "git init failed");

    ui.send(Message::ProjectGit { project_id });

    let overview = loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::GitOverview {
                project_id: id,
                overview,
            }) => {
                assert_eq!(id, project_id);
                break overview;
            }
            Ok(_) => continue,
            Err(_) => panic!("git was never answered"),
        }
    };
    let overview = overview.expect("the project is a repository");
    assert_eq!(
        overview.head,
        ubiq_proto::git::GitHead::Unborn("main".into())
    );

    while let Ok(message) = other.from_host().recv_timeout(Duration::from_millis(200)) {
        assert!(
            !matches!(message, Message::GitOverview { .. }),
            "an overview reached a window that did not ask for it"
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

#[test]
fn a_diff_answers_the_window_that_asked_and_no_other() {
    let (hub, ui) = coordinator();
    let other = hub.connect();
    let (project_id, path) = a_project(&ui);

    // A scratch repository, committed and then changed — the diff has to come from version
    // control rather than from anything the host remembers about the read.
    scratch_git(&path, &["init", "-q", "-b", "main"]);
    std::fs::write(path.join("file.txt"), b"one\ntwo\n").unwrap();
    scratch_git(&path, &["add", "."]);
    scratch_git(&path, &["commit", "-q", "-m", "first"]);
    std::fs::write(path.join("file.txt"), b"one\nchanged\n").unwrap();

    ui.send(Message::DiffProjectFile {
        project_id,
        rel_path: "file.txt".to_string(),
        base: DiffBase::Head,
    });

    let diff = loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::ProjectFileDiffed { diff, rel_path, .. }) => {
                assert_eq!(rel_path, "file.txt");
                break diff;
            }
            Ok(Message::ProjectFileError { error, .. }) => panic!("the diff failed: {error:?}"),
            Ok(_) => continue,
            Err(_) => panic!("the diff was never answered"),
        }
    };

    assert_eq!(diff.base, DiffBase::Head);
    assert_eq!(diff.hunks.len(), 1, "{diff:?}");
    let added: Vec<&str> = diff.hunks[0]
        .rows
        .iter()
        .filter(|row| row.kind == DiffRowKind::Added)
        .map(|row| row.text.as_str())
        .collect();
    assert_eq!(added, vec!["changed"]);

    // The file family answers one window, and a diff is no exception.
    while let Ok(message) = other.from_host().recv_timeout(Duration::from_millis(200)) {
        assert!(
            !matches!(message, Message::ProjectFileDiffed { .. }),
            "a diff reached a window that did not ask for it"
        );
    }
}

#[test]
fn a_diff_in_a_project_with_no_version_control_is_refused() {
    let (_hub, ui) = coordinator();
    let (project_id, path) = a_project(&ui);
    std::fs::write(path.join("alone.txt"), b"alone\n").unwrap();

    ui.send(Message::DiffProjectFile {
        project_id,
        rel_path: "alone.txt".to_string(),
        base: DiffBase::Head,
    });

    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::ProjectFileError {
                error, rel_path, ..
            }) => {
                assert_eq!(rel_path, "alone.txt");
                assert!(matches!(error, FileError::Refused(_)), "answered {error:?}");
                return;
            }
            Ok(Message::ProjectFileDiffed { .. }) => {
                panic!("a folder with no version control was diffed")
            }
            Ok(_) => continue,
            Err(_) => panic!("the diff was never answered"),
        }
    }
}

/// One git command in a scratch folder, with the machine's own configuration kept out of it.
fn scratch_git(dir: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Ubiq")
        .env("GIT_AUTHOR_EMAIL", "ubiq@example.invalid")
        .env("GIT_COMMITTER_NAME", "Ubiq")
        .env("GIT_COMMITTER_EMAIL", "ubiq@example.invalid")
        .args(args)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// ── the work family over the bus ────────────────────────────────────

#[test]
fn a_work_listing_answers_the_fixture_for_the_project_that_asked() {
    let (_hub, ui) = coordinator();
    let (project_id, _path) = a_project(&ui);

    ui.send(Message::ListWork { project_id });

    let (sessions, agents, tasks) = expect_work_list(&ui, project_id);
    // The seed a project that never wrote a `tasks.toml` starts with, whole and in one reply: the
    // graph draws a card and the session it names in the same frame.
    assert_eq!(sessions.len(), 5);
    assert_eq!(agents.len(), 11);
    assert_eq!(tasks.len(), 10);
}

#[test]
fn a_task_is_created_and_then_changed_over_the_bus() {
    let (_hub, ui) = coordinator();
    let (project_id, _path) = a_project(&ui);

    ui.send(Message::CreateTask {
        project_id,
        title: "Name the events the host already knows".to_string(),
        session: None,
    });
    let created = expect_task_created(&ui);
    assert_eq!(created.title, "Name the events the host already knows");

    ui.send(Message::UpdateTask {
        project_id,
        task_id: created.id,
        title: Some("Name the events".to_string()),
        description: Some("## Why\n\nthe poll is the wrong shape".to_string()),
        priority: None,
        shape: None,
    });

    let changed = expect_task_changed(&ui);
    assert_eq!(
        changed.id, created.id,
        "the same card, whole rather than a diff"
    );
    assert_eq!(changed.title, "Name the events");
    assert_eq!(changed.description, "## Why\n\nthe poll is the wrong shape");
}

/// Work for a project the catalogue does not hold is refused before the store is touched.
///
/// Not a formality: a `tasks.toml` written under an id no record names would be collected as an
/// orphan at the next boot, so the write must never happen at all.
#[test]
fn work_for_a_project_the_catalogue_does_not_hold_is_refused() {
    let (_hub, ui) = coordinator();
    let project_id = ProjectId::generate();

    ui.send(Message::ListWork { project_id });

    let (id, error) = expect_work_error(&ui);
    assert_eq!(id, project_id);
    assert!(error.contains("no such project"), "said {error:?}");
}

#[test]
fn work_for_a_forgotten_project_is_refused_the_same_way() {
    let (_hub, ui) = coordinator();
    let (project_id, _path) = a_project(&ui);
    ui.send(Message::ListWork { project_id });
    expect_work_list(&ui, project_id);

    ui.send(Message::ForgetProject { project_id });
    ui.send(Message::ListWork { project_id });

    let (id, _error) = expect_work_error(&ui);
    assert_eq!(id, project_id);
}

#[test]
fn a_task_change_reaches_only_the_window_that_asked() {
    let (hub, ui) = coordinator();
    let other = hub.connect();
    let (project_id, _path) = a_project(&ui);

    ui.send(Message::CreateTask {
        project_id,
        title: "one for the asker".to_string(),
        session: None,
    });
    let created = expect_task_created(&ui);
    ui.send(Message::UpdateTask {
        project_id,
        task_id: created.id,
        title: Some("still for the asker".to_string()),
        description: None,
        priority: None,
        shape: None,
    });
    assert_eq!(expect_task_changed(&ui).title, "still for the asker");

    // A project is open in exactly one window at a time, so the window that asked is the only one
    // drawing that project's work. Nothing in the family is broadcast.
    while let Ok(message) = other.from_host().recv_timeout(Duration::from_millis(200)) {
        assert!(
            !matches!(
                message,
                Message::WorkList { .. }
                    | Message::TaskCreated { .. }
                    | Message::TaskChanged { .. }
                    | Message::TaskDeleted { .. }
                    | Message::AgentChanged { .. }
                    | Message::WorkError { .. }
            ),
            "work reached a window that did not ask for it: {message:?}"
        );
    }
}

/// The interface's workarea is reserved where the layout says, and the host leaves it empty.
///
/// Two facts in one test, because they are the whole of the rule: the path the snapshot carries is
/// `projects/<ulid>/ui/`, it exists by the time the interface is told about it, and nothing the
/// host does afterwards puts anything in it. The interface never composes this path itself.
#[test]
fn a_project_arrives_with_a_workarea_the_host_reserves_and_leaves_alone() {
    let (_hub, ui, root) = coordinator_on_disk();
    let folder = tempfile::TempDir::new().unwrap();
    let project = add_project(&ui, folder.path());

    let expected = root.join("projects").join(project.to_string()).join("ui");
    let snapshot = expect_project_list(&ui)
        .into_iter()
        .find(|p| p.id() == project)
        .expect("the project is in the listing");
    assert_eq!(
        std::path::Path::new(&snapshot.workarea),
        expected,
        "the workarea sits beside the project's own files under Ubiq's config root"
    );
    assert!(
        expected.is_dir(),
        "and the host has already made it, because the interface is told a path and not a maybe"
    );

    // The host writes a project's tasks and its view state; none of it lands in here.
    ui.send(Message::ListWork {
        project_id: project,
    });
    let _ = expect_work_list(&ui, project);
    let path = root
        .join("projects")
        .join(project.to_string())
        .join("tasks.toml");
    wait_for_body(&path, "[[task]]");
    assert_eq!(
        std::fs::read_dir(&expected).unwrap().count(),
        0,
        "the host reserves the workarea and never writes inside it"
    );
}

fn expect_project_list(ui: &Client) -> Vec<ubiq_proto::projects::ProjectSnapshot> {
    ui.send(Message::ListProjects);
    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::ProjectList { projects }) => return projects,
            Ok(_) => continue,
            Err(_) => panic!("the catalogue was never listed"),
        }
    }
}

/// A project's first look at the board writes its tasks down, and an edit lands in the same file.
///
/// The whole path in one test: a window asks over the bus, the host seeds the fixture, the file
/// store writes it to `projects/<ulid>/tasks.toml`, and a later `UpdateTask` is in those bytes
/// afterwards. Everything else here runs on the memory stores, so this is the only thing that
/// proves the layout and the format are what the documentation says.
#[test]
fn a_first_listing_writes_the_project_tasks_where_the_layout_says() {
    let (_hub, ui, root) = coordinator_on_disk();
    let folder = tempfile::TempDir::new().unwrap();
    let project = add_project(&ui, folder.path());

    ui.send(Message::ListWork {
        project_id: project,
    });
    let (_, _, tasks) = expect_work_list(&ui, project);
    assert_eq!(
        tasks.len(),
        10,
        "the fixture is what a new project starts on"
    );

    let path = root
        .join("projects")
        .join(project.to_string())
        .join("tasks.toml");
    let body = wait_for_body(&path, "[[task]]");
    assert!(
        body.contains("version = 1"),
        "the envelope carries the version a migration would read: {body}"
    );
    assert_eq!(
        body.matches("[[task]]").count(),
        10,
        "one array entry per task"
    );
    assert!(
        body.contains("## Why"),
        "and the seeded markdown is in the file as it was written: {body}"
    );

    ui.send(Message::UpdateTask {
        project_id: project,
        task_id: tasks[0].id,
        title: Some("Renamed over the bus".to_string()),
        description: None,
        priority: None,
        shape: None,
    });
    let changed = expect_task_changed(&ui);
    assert_eq!(changed.title, "Renamed over the bus");

    // Durable, not merely answered — the difference this whole half of the change is about.
    wait_for_body(&path, "Renamed over the bus");
}

/// Wait for bytes the coordinator writes on its own thread, after answering on the bus.
fn wait_for_body(path: &std::path::Path, needle: &str) -> String {
    let deadline = std::time::Instant::now() + PATIENCE;
    let mut last = String::new();
    while std::time::Instant::now() < deadline {
        last = std::fs::read_to_string(path).unwrap_or_default();
        if last.contains(needle) {
            return last;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("{needle} never reached {}: {last}", path.display());
}

#[test]
fn settings_round_trip_on_both_layers() {
    let (_hub, ui) = coordinator();
    let ui_blob = r#"{"schema":1,"explorer_preview":false,"markdown_open":"source"}"#;
    let host_blob = serde_json::to_string(&HostSettings::default()).unwrap();

    ui.send(Message::SetSettings {
        layer: SettingsLayer::Ui,
        value: ui_blob.to_string(),
    });
    ui.send(Message::SetSettings {
        layer: SettingsLayer::Host,
        value: host_blob.clone(),
    });
    ui.send(Message::GetSettings {
        layer: SettingsLayer::Ui,
    });
    ui.send(Message::GetSettings {
        layer: SettingsLayer::Host,
    });

    let mut ui_back = None;
    let mut host_back = None;
    while ui_back.is_none() || host_back.is_none() {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::Settings {
                layer: SettingsLayer::Ui,
                value,
            }) => ui_back = Some(value),
            Ok(Message::Settings {
                layer: SettingsLayer::Host,
                value,
            }) => host_back = Some(value),
            Ok(Message::HostInfo { .. }) => {}
            Ok(Message::SettingsError { error, .. }) => panic!("settings failed: {error}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    assert_eq!(ui_back.unwrap().as_deref(), Some(ui_blob));
    let host = host_back.unwrap().expect("host settings were stored");
    let parsed: HostSettings = serde_json::from_str(&host).unwrap();
    assert_eq!(parsed, HostSettings::default());
}

#[test]
fn a_host_blob_this_build_cannot_read_is_an_error() {
    let (_hub, ui) = coordinator();
    ui.send(Message::SetSettings {
        layer: SettingsLayer::Host,
        value: r#"{"schema":99}"#.to_string(),
    });

    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::SettingsError {
                layer: SettingsLayer::Host,
                error,
            }) => {
                assert!(error.contains("99"), "{error}");
                return;
            }
            Ok(Message::HostInfo { .. }) => continue,
            other => panic!("expected SettingsError, got {other:?}"),
        }
    }
}

fn expect_work_list(
    ui: &Client,
    project_id: ProjectId,
) -> (
    Vec<ubiq_proto::work::WorkSession>,
    Vec<ubiq_proto::work::WorkAgent>,
    Vec<ubiq_proto::work::TaskRecord>,
) {
    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::WorkList {
                project_id: id,
                sessions,
                agents,
                tasks,
            }) => {
                assert_eq!(id, project_id);
                return (sessions, agents, tasks);
            }
            Ok(Message::WorkError { error, .. }) => panic!("the listing failed: {error}"),
            Ok(_) => continue,
            Err(_) => panic!("the work was never listed"),
        }
    }
}

fn expect_task_created(ui: &Client) -> ubiq_proto::work::TaskRecord {
    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::TaskCreated { task, .. }) => return task,
            Ok(Message::WorkError { error, .. }) => panic!("the task was refused: {error}"),
            Ok(_) => continue,
            Err(_) => panic!("the task was never created"),
        }
    }
}

fn expect_task_changed(ui: &Client) -> ubiq_proto::work::TaskRecord {
    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::TaskChanged { task, .. }) => return task,
            Ok(Message::WorkError { error, .. }) => panic!("the change was refused: {error}"),
            Ok(_) => continue,
            Err(_) => panic!("the task was never changed"),
        }
    }
}

fn expect_work_error(ui: &Client) -> (ProjectId, String) {
    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::WorkError {
                project_id, error, ..
            }) => return (project_id, error),
            Ok(Message::WorkList { .. }) => panic!("the work was answered rather than refused"),
            Ok(_) => continue,
            Err(_) => panic!("the request was neither answered nor refused"),
        }
    }
}

#[test]
fn a_shell_pane_is_a_login_shell() {
    let (_hub, ui) = coordinator();
    let pane_id = spawn(&ui, "/bin/sh", &[]);

    // A login shell's argv0 is its name with a `-` on it, which is the whole reason
    // `.zprofile`/`.profile` run at all — without it a pane's `PATH` is not the user's.
    ui.send(Message::TerminalInput {
        pane_id,
        bytes: b"echo argv0=$0\n".to_vec(),
    });
    wait_for_output(&ui, "argv0=-sh");

    ui.send(Message::CloseWorkspace { pane_id });
}

#[test]
fn a_program_with_arguments_is_started_plainly() {
    let (_hub, ui) = coordinator();
    // The login prefix is a shell's business: anything handed a command line is started as itself,
    // so a harness never sees a `-` on its own argv0.
    spawn(&ui, "/bin/sh", &["-c", "echo argv0=$0"]);

    wait_for_output(&ui, "argv0=/bin/sh");
}

#[test]
fn the_shell_list_offers_what_is_installed_with_the_default_marked() {
    let (_hub, ui) = coordinator();
    ui.send(Message::ListShells);

    let shells = loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::ShellList { shells }) => break shells,
            Ok(Message::HostInfo { .. }) => continue,
            other => panic!("expected the shell list, got {other:?}"),
        }
    };

    assert!(
        !shells.is_empty(),
        "a machine with no shell cannot run this"
    );
    assert_eq!(
        shells.iter().filter(|shell| shell.is_default).count(),
        1,
        "exactly one row is the default: {shells:?}"
    );
    for shell in &shells {
        assert!(
            std::path::Path::new(&shell.program).is_file(),
            "{} is offered but not there",
            shell.program
        );
    }
}

// ── the conversation family: P3's pending stage ─────────────────────
//
// `"opencode"` is a real harness id — `is_agent_type` resolves it by name alone, so a pending
// agent registers for it whether or not the `opencode` binary is actually on this machine — but
// nothing here ever spawns it, so the tests hold regardless of what is installed.

fn start_conversation(
    ui: &Client,
    project_id: ProjectId,
    agent_type: &str,
    account: Option<&str>,
) -> AgentId {
    let agent_id = AgentId::generate();
    ui.send(Message::StartConversation {
        agent_id,
        project_id,
        session_id: SessionId::generate(),
        rel_path: None,
        agent_type: agent_type.to_string(),
        account: account.map(str::to_string),
        name: None,
    });
    agent_id
}

/// The immediate answer to `StartConversation`: registered, with a loader, before any harness
/// exists. Answers whether this harness takes a second turn.
fn expect_conversation_started(ui: &Client, agent_id: AgentId) -> bool {
    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::ConversationStarted {
                agent,
                accepts_input,
                ..
            }) if agent.id == agent_id => return accepts_input,
            Ok(Message::ConversationError {
                agent_id: id,
                error,
            }) if id == agent_id => {
                panic!("the conversation was refused: {error}")
            }
            Ok(_) => continue,
            Err(_) => panic!("ConversationStarted never arrived"),
        }
    }
}

/// The model-discovery thread's own message — always the first thing a pending agent says,
/// always seq 1, since nothing else can address a brand-new `agent_id` before the window has even
/// heard of it. Asserts only the shape the doc promises regardless of what discovery found: one
/// `model` option, never zero.
fn expect_model_config_options(ui: &Client, agent_id: AgentId) {
    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::ConversationUpdate {
                agent_id: id,
                seq,
                update,
            }) if id == agent_id => {
                assert_eq!(seq, 1, "the discovery thread's message is always the first");
                let ConvUpdate::ConfigOptions(options) = *update else {
                    panic!("expected ConfigOptions, got {update:?}");
                };
                assert_eq!(
                    options.len(),
                    1,
                    "one option even when a harness's list could not be read"
                );
                assert_eq!(options[0].id, "model");
                return;
            }
            Ok(_) => continue,
            Err(_) => panic!("the model picker never arrived"),
        }
    }
}

fn expect_conversation_error(ui: &Client, agent_id: AgentId) -> String {
    loop {
        match ui.from_host().recv_timeout(PATIENCE) {
            Ok(Message::ConversationError {
                agent_id: id,
                error,
            }) if id == agent_id => {
                return error;
            }
            Ok(_) => continue,
            Err(_) => panic!("the launch failure was never reported"),
        }
    }
}

#[test]
fn a_conversation_is_registered_and_its_models_discovered_before_any_harness_launches() {
    let (_hub, ui) = coordinator();
    let (project_id, _path) = a_project(&ui);

    let agent_id = start_conversation(&ui, project_id, "opencode", None);
    let accepts_input = expect_conversation_started(&ui, agent_id);
    assert!(!accepts_input, "opencode takes no second turn");
    expect_model_config_options(&ui, agent_id);

    // Registered at once: the project's own listing already carries it, with nothing spawned to
    // produce this — proof enough over the bus that registration did not wait on a process.
    ui.send(Message::ListWork { project_id });
    let (_, agents, _) = expect_work_list(&ui, project_id);
    assert!(
        agents.iter().any(|a| a.id == agent_id),
        "the pending agent is not in the project's list"
    );
}

#[test]
fn set_agent_config_on_a_pending_agent_is_accepted_silently() {
    let (_hub, ui) = coordinator();
    let (project_id, _path) = a_project(&ui);

    let agent_id = start_conversation(&ui, project_id, "opencode", None);
    expect_conversation_started(&ui, agent_id);
    expect_model_config_options(&ui, agent_id);

    ui.send(Message::SetAgentConfig {
        agent_id,
        config_id: "model".to_string(),
        value: "some-model".to_string(),
    });

    // A pick on a pending agent is only remembered for the eventual launch — no conversation
    // exists yet to answer through, and nothing here says otherwise.
    assert!(
        ui.from_host()
            .recv_timeout(Duration::from_millis(300))
            .is_err(),
        "a config pick on a pending agent must not answer anything"
    );
}

#[test]
fn a_launch_that_fails_retracts_the_agent_it_registered() {
    let (_hub, ui) = coordinator();
    let (project_id, _path) = a_project(&ui);

    // An account nothing in this fresh root knows about — resolution refuses the run before any
    // process is spawned, which is true whatever harnesses this machine happens to have.
    let agent_id = start_conversation(&ui, project_id, "opencode", Some("no-such-account"));
    expect_conversation_started(&ui, agent_id);
    expect_model_config_options(&ui, agent_id);

    ui.send(Message::PromptAgent {
        agent_id,
        text: "hello".to_string(),
    });
    let error = expect_conversation_error(&ui, agent_id);
    assert!(error.contains("no-such-account"), "said {error:?}");

    // The failure retracts what registration made visible, on the same terms a live agent's own
    // end does.
    ui.send(Message::ListWork { project_id });
    let (_, agents, _) = expect_work_list(&ui, project_id);
    assert!(
        !agents.iter().any(|a| a.id == agent_id),
        "a failed launch must not leave a dead agent in the list"
    );
}
