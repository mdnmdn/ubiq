//! The search family, against a real drone.
//!
//! The first three drive `Relay` in-process over a loopback pair, the same way `session.rs` does
//! — what is under test is the translation between `Query`/`Filter` and a shelled-out tool, not
//! process lifecycle. The last drives the real binary instead, because what is under test there is
//! `PATH` itself: an in-process `Relay` shares this test binary's own environment with every other
//! test in it (`search::probe` caches its answer for the life of the process), and could not be
//! starved of a tool without starving them all.
//!
//! `rg` and `ag` may or may not be installed on the machine running these tests; the three
//! in-process tests skip themselves, with a reason, when `ubiq_drone::search::probe()` finds
//! neither. The fourth always runs — a machine with no search tool at all is exactly its subject.

use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use ubiq_drone::relay::{Relay, Root};
use ubiq_host::carrier::{self, NoCloser};
use ubiq_proto::bus;
use ubiq_proto::carrier::welcome;
use ubiq_proto::ids::SearchId;
use ubiq_proto::messages::Message;
use ubiq_proto::search::{Batch, Filter, Query, Scope, SearchError};
use ubiq_proto::wire;

/// How long a test waits for one expected message. Generous: a cold pseudo-terminal on a loaded
/// machine is slow, and a flaky test is worse than a slow one.
const PATIENCE: Duration = Duration::from_secs(20);

/// A client attached to a relay over a real duplex stream — the same shape `session.rs`'s own
/// `Session` is, duplicated rather than shared: each test file in this crate is self-contained.
struct Session {
    client: bus::Client,
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
    fn open(roots: Vec<Root>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let address = listener.local_addr().expect("the bound address");
        let far = TcpStream::connect(address).expect("dialling the relay");
        let (near, _) = listener.accept().expect("accepting the client");

        let (hub, host) = bus::hub();
        let relay = std::thread::spawn(move || Relay::new(roots).run(host));
        let pump = std::thread::spawn(move || {
            let reader = near.try_clone().expect("the read half");
            carrier::pump(Box::new(reader), Box::new(near), Box::new(NoCloser), hub);
        });

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

    /// Attach, list the one project a test seeded, and answer with its id.
    fn attach_and_list_one_project(&self) -> ubiq_proto::ids::ProjectId {
        self.wait_for("HostInfo", |message| {
            matches!(message, Message::HostInfo { .. }).then_some(())
        });
        self.say(Message::ListProjects);
        self.wait_for("ProjectList", |message| match message {
            Message::ProjectList { projects } => Some(projects[0].id()),
            _ => None,
        })
    }
}

/// A search with every option left at its plainest.
fn plain_query(text: &str) -> Query {
    Query {
        text: text.to_string(),
        case_sensitive: false,
        whole_word: false,
        regex: false,
    }
}

/// `None` if this machine has nothing on `PATH` this crate's own probe would shell out to — the
/// three in-process tests below have nothing to drive in that case, and skip themselves.
fn require_a_tool() -> bool {
    if ubiq_drone::search::probe().is_none() {
        eprintln!("skipping: no search tool (rg, ag or grep) on this machine's PATH");
        return false;
    }
    true
}

#[test]
fn a_search_over_a_seeded_root_finds_a_known_line_with_its_range() {
    if !require_a_tool() {
        return;
    }
    let project = tempfile::tempdir().expect("a project folder");
    std::fs::write(
        project.path().join("hello.rs"),
        b"one\ntwo\nthe needle is here\nfour\n",
    )
    .expect("seeding a file");

    let session = Session::open(vec![Root::new(project.path().to_path_buf())]);
    let project_id = session.attach_and_list_one_project();

    let search_id = SearchId::generate();
    session.say(Message::SearchProject {
        project_id,
        search_id,
        query: plain_query("needle"),
        scope: Scope::Files,
        filter: Filter::default(),
    });

    let (line, ranges) = session.wait_for("a match for hello.rs", |message| match message {
        Message::SearchMatches {
            search_id: sid,
            batch: Batch::Files(files),
            ..
        } if *sid == search_id => files
            .iter()
            .find(|file| file.rel_path == "hello.rs")
            .map(|file| (file.lines[0].line, file.lines[0].ranges.clone())),
        _ => None,
    });
    assert_eq!(line, 3, "the needle is on the third line");
    assert_eq!(
        ranges,
        vec![(4, 10)],
        "the byte range of \"needle\" within its line"
    );

    session.wait_for("SearchFinished", |message| match message {
        Message::SearchFinished { search_id: sid, .. } if *sid == search_id => Some(()),
        _ => None,
    });
}

/// `regex: false` must reach the shelled-out tool as a literal flag (`-F`/`--literal`), not just a
/// hope that the metacharacters happen to parse as themselves.
#[test]
fn a_literal_query_with_metacharacters_and_a_space_is_not_treated_as_regex() {
    if !require_a_tool() {
        return;
    }
    let project = tempfile::tempdir().expect("a project folder");
    // As a *regex*, "test(value) [x]" means something else entirely — a group and a character
    // class, neither of which appears literally in the pattern's own matched text. A hit only
    // shows up here at all if the tool was told to treat this as a literal string.
    let needle = "test(value) [x]";
    std::fs::write(
        project.path().join("literal.txt"),
        format!("before {needle} after\n"),
    )
    .expect("seeding a file");

    let session = Session::open(vec![Root::new(project.path().to_path_buf())]);
    let project_id = session.attach_and_list_one_project();

    let search_id = SearchId::generate();
    session.say(Message::SearchProject {
        project_id,
        search_id,
        query: Query {
            text: needle.to_string(),
            case_sensitive: true,
            whole_word: false,
            regex: false,
        },
        scope: Scope::Files,
        filter: Filter::default(),
    });

    session.wait_for("a literal match", |message| match message {
        Message::SearchMatches {
            search_id: sid,
            batch: Batch::Files(files),
            ..
        } if *sid == search_id => files
            .iter()
            .find(|file| file.rel_path == "literal.txt")
            .map(drop),
        _ => None,
    });

    session.wait_for("SearchFinished", |message| match message {
        Message::SearchFinished { search_id: sid, .. } if *sid == search_id => Some(()),
        _ => None,
    });
}

/// A search over enough files that it is still running when the cancel arrives stops early,
/// rather than running to completion regardless.
///
/// Timing-dependent, like every "did the cancel actually land in time" test: the file count is
/// chosen to keep even a fast tool busy for longer than one loopback round trip, on the same
/// reasoning `PATIENCE` states above for a slow machine. What is not in question is whether the
/// drone hangs — `wait_for` panics on its own if `SearchFinished` never arrives.
#[test]
fn a_cancel_stops_a_running_search() {
    if !require_a_tool() {
        return;
    }
    let project = tempfile::tempdir().expect("a project folder");
    const FILES: usize = 20_000;
    for at in 0..FILES {
        let dir = project.path().join(format!("{:03}", at / 200));
        std::fs::create_dir_all(&dir).expect("a subdirectory");
        std::fs::write(dir.join(format!("f{at}.txt")), b"needle\n").expect("seeding a file");
    }

    let session = Session::open(vec![Root::new(project.path().to_path_buf())]);
    let project_id = session.attach_and_list_one_project();

    let search_id = SearchId::generate();
    session.say(Message::SearchProject {
        project_id,
        search_id,
        query: plain_query("needle"),
        scope: Scope::Files,
        filter: Filter::default(),
    });
    session.say(Message::CancelSearch {
        project_id,
        search_id,
    });

    let mut files_reported = 0usize;
    session.wait_for("SearchFinished", |message| match message {
        Message::SearchMatches {
            search_id: sid,
            batch: Batch::Files(files),
            ..
        } if *sid == search_id => {
            files_reported += files.len();
            None
        }
        Message::SearchFinished { search_id: sid, .. } if *sid == search_id => Some(()),
        _ => None,
    });

    assert!(
        files_reported < FILES,
        "a cancel sent right behind the search should stop it well short of every file; got \
         {files_reported} of {FILES}"
    );
}

/// A scope a drone cannot serve answers empty rather than with a refusal — nothing failed, there
/// was simply nothing here to look at.
#[test]
fn a_scope_beyond_files_answers_finished_with_nothing_searched() {
    let project = tempfile::tempdir().expect("a project folder");
    let session = Session::open(vec![Root::new(project.path().to_path_buf())]);
    let project_id = session.attach_and_list_one_project();

    let search_id = SearchId::generate();
    session.say(Message::SearchProject {
        project_id,
        search_id,
        query: plain_query("anything"),
        scope: Scope::Project,
        filter: Filter::default(),
    });

    let searched = session.wait_for("SearchFinished", |message| match message {
        Message::SearchFinished {
            search_id: sid,
            searched,
            ..
        } if *sid == search_id => Some(searched.clone()),
        _ => None,
    });
    assert!(
        searched.is_empty(),
        "a drone answers only Scope::Files: {searched:?}"
    );
}

/// A machine with no search tool at all answers with a `SearchError` naming what it looked for,
/// never with silence — driven against the real binary, `PATH` and all, because that is the one
/// fact this test needs to control that an in-process `Relay` (sharing this test binary's own,
/// already-probed environment) cannot give it.
#[test]
fn no_search_tool_on_path_answers_a_search_error_naming_the_tools() {
    let project = tempfile::tempdir().expect("a project folder");
    let empty_path = tempfile::tempdir().expect("a PATH with nothing on it");

    let mut child = Command::new(env!("CARGO_BIN_EXE_ubiq-drone"))
        .arg("--stdio")
        .arg("--root")
        .arg(project.path())
        .env("PATH", empty_path.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("starting a drone with an empty PATH");

    let mut stdin = child.stdin.take().expect("the drone's stdin");
    let mut stdout = child.stdout.take().expect("the drone's stdout");

    let identity = welcome(&mut stdout, &mut stdin, "0.1.0").expect("an accepted drone");
    assert!(
        !identity.has("search"),
        "an empty PATH means no search capability to advertise: {:?}",
        identity.capabilities
    );

    match wire::read_frame(&mut stdout).expect("a frame") {
        Message::HostInfo { .. } => {}
        other => panic!("an accepted drone greets its client with HostInfo, not {other:?}"),
    }

    wire::write_frame(&mut stdin, &Message::ListProjects).expect("writing ListProjects");
    let project_id = match wire::read_frame(&mut stdout).expect("a frame") {
        Message::ProjectList { projects } => projects[0].id(),
        other => panic!("expected ProjectList, got {other:?}"),
    };

    let search_id = SearchId::generate();
    wire::write_frame(
        &mut stdin,
        &Message::SearchProject {
            project_id,
            search_id,
            query: plain_query("anything"),
            scope: Scope::Files,
            filter: Filter::default(),
        },
    )
    .expect("writing SearchProject");

    match wire::read_frame(&mut stdout).expect("a frame") {
        Message::SearchError {
            search_id: sid,
            error: SearchError::Walk(reason),
            ..
        } => {
            assert_eq!(sid, search_id);
            for tool in ["rg", "ag", "grep"] {
                assert!(
                    reason.contains(tool),
                    "the refusal should name what it looked for: {reason}"
                );
            }
        }
        other => panic!("expected SearchError, got {other:?}"),
    }

    let _ = child.kill();
    let _ = child.wait();
}
