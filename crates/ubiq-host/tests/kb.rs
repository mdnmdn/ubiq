//! A project's knowledge base: the source list round-tripping through its file, the glob a
//! listing obeys, and the two refusals that must never reach a thread.
//!
//! Modelled on `files.rs`: what is worth testing here is not the walk itself — `files::listing`
//! and `files::contents` already have their own suite — it is what `crate::kb` adds on top of it:
//! where a source's list lives, and the boundary a listing is filtered through before it leaves.

use std::fs;
use std::path::Path;
use std::time::Duration;

use tempfile::TempDir;
use ubiq_host::files::{self, Files};
use ubiq_host::kb::{Kb, ops};
use ubiq_proto::bus::{self, Mailbox, To};
use ubiq_proto::files::FileError;
use ubiq_proto::ids::{KbSourceId, ProjectId};
use ubiq_proto::kb::{KbAccess, KbOrigin, KbSource, KbSourceState};
use ubiq_proto::messages::Message;

/// Long enough for a worker thread on a loaded machine, never hit in the ordinary case.
const PATIENCE: Duration = Duration::from_secs(5);

/// No test here exercises `KbStore::Project`, so no source needs a real project folder — this is
/// the empty path every other call site falls back to as well.
fn no_project_path() -> &'static Path {
    Path::new("")
}

/// A client with nothing behind it but a mailbox that delivers to it — everything these tests
/// need from the bus, and no coordinator to run one.
fn harness() -> (bus::Client, Mailbox, bus::Hub) {
    let (hub, host) = bus::hub();
    let client = hub.connect();
    let mailbox = host.mailbox(To::Client(client.id()));
    (client, mailbox, hub)
}

fn folder_source(filter: &str, path: &std::path::Path) -> KbSource {
    KbSource {
        id: KbSourceId::generate(),
        name: "docs".to_string(),
        origin: KbOrigin::Folder {
            path: path.to_string_lossy().into_owned(),
        },
        filter: filter.to_string(),
        access: Default::default(),
    }
}

// ── the source list ─────────────────────────────────────────────────

#[test]
fn sources_round_trip_through_the_store() {
    let config = TempDir::new().unwrap();
    let folder = TempDir::new().unwrap();
    let (_client, mailbox, _hub) = harness();
    let project = ProjectId::generate();
    let source = folder_source("", folder.path());

    let kb = Kb::new(config.path().to_path_buf());
    let statuses = kb.set_sources(project, vec![source.clone()], mailbox, no_project_path());
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].state, KbSourceState::Ready, "the folder exists");

    // A fresh `Kb` over the same config root — nothing but the file on disk connects them — reads
    // the same list back.
    let reopened = Kb::new(config.path().to_path_buf());
    let reread = reopened.sources(project, no_project_path());
    assert_eq!(reread.len(), 1);
    assert_eq!(reread[0].source.id, source.id);
    assert_eq!(reread[0].source.name, "docs");
    assert_eq!(reread[0].state, KbSourceState::Ready);
}

#[test]
fn a_project_that_never_configured_a_kb_answers_an_empty_list() {
    let config = TempDir::new().unwrap();
    let kb = Kb::new(config.path().to_path_buf());
    assert!(
        kb.sources(ProjectId::generate(), no_project_path())
            .is_empty()
    );
}

#[test]
fn an_unknown_project_or_source_is_refused_before_a_thread_sees_it() {
    let config = TempDir::new().unwrap();
    let folder = TempDir::new().unwrap();
    let (_client, mailbox, _hub) = harness();
    let kb = Kb::new(config.path().to_path_buf());
    let project = ProjectId::generate();

    // A project that has never written a list: `find` is the coordinator's own gate before a job
    // ever reaches `Files`, and it has nothing to look up.
    assert!(kb.find(project, KbSourceId::generate()).is_none());

    // A project with a list, asked about a source it does not hold.
    let source = folder_source("", folder.path());
    kb.set_sources(project, vec![source.clone()], mailbox, no_project_path());
    assert!(kb.find(project, KbSourceId::generate()).is_none());
    assert!(kb.find(project, source.id).is_some());

    // A source id that exists, but for a different project entirely.
    assert!(kb.find(ProjectId::generate(), source.id).is_none());
}

// ── listing, through the worker a `KbTree` job actually runs on ────

#[test]
fn a_filter_hides_a_file_it_does_not_name_from_a_kb_listing() {
    let base = TempDir::new().unwrap();
    fs::write(base.path().join("notes.md"), b"# hi").unwrap();
    fs::write(base.path().join("image.png"), b"\x89PNG").unwrap();
    fs::create_dir(base.path().join("empty")).unwrap();

    let source = folder_source("*.md", base.path());
    let (client, mailbox, _hub) = harness();
    let files = Files::start();
    files.submit(files::Job {
        kind: files::JobKind::Kb {
            project_id: ProjectId::generate(),
            source,
            base: base.path().to_path_buf(),
            request: files::Request::Tree {
                rel_path: String::new(),
                depth: 1,
            },
        },
        reply_to: mailbox,
    });

    let message = client.from_host().recv_timeout(PATIENCE).unwrap();
    let Message::KbTreeListing { listings, .. } = message else {
        panic!("expected a kb tree listing, got {message:?}")
    };
    let names: Vec<&str> = listings[0]
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert!(names.contains(&"notes.md"), "{names:?}");
    assert!(!names.contains(&"image.png"), "{names:?}");
    // A folder is never filtered out, matching or not: it is simply empty when it is opened.
    assert!(names.contains(&"empty"), "{names:?}");
}

#[test]
fn an_empty_filter_admits_everything_in_a_kb_listing() {
    let base = TempDir::new().unwrap();
    fs::write(base.path().join("a.md"), b"a").unwrap();
    fs::write(base.path().join("b.bin"), b"b").unwrap();

    let source = folder_source("", base.path());
    let (client, mailbox, _hub) = harness();
    let files = Files::start();
    files.submit(files::Job {
        kind: files::JobKind::Kb {
            project_id: ProjectId::generate(),
            source,
            base: base.path().to_path_buf(),
            request: files::Request::Tree {
                rel_path: String::new(),
                depth: 1,
            },
        },
        reply_to: mailbox,
    });

    let message = client.from_host().recv_timeout(PATIENCE).unwrap();
    let Message::KbTreeListing { listings, .. } = message else {
        panic!("expected a kb tree listing, got {message:?}")
    };
    assert_eq!(listings[0].entries.len(), 2);
}

#[test]
fn a_kb_read_outside_the_source_is_refused() {
    let base = TempDir::new().unwrap();
    fs::write(base.path().join("inside.md"), b"ok").unwrap();

    let source = folder_source("", base.path());
    let (client, mailbox, _hub) = harness();
    let files = Files::start();
    files.submit(files::Job {
        kind: files::JobKind::Kb {
            project_id: ProjectId::generate(),
            source,
            base: base.path().to_path_buf(),
            request: files::Request::Read {
                rel_path: "../outside".to_string(),
                max_bytes: None,
            },
        },
        reply_to: mailbox,
    });

    let message = client.from_host().recv_timeout(PATIENCE).unwrap();
    match message {
        Message::KbFileError { error, .. } => {
            assert!(matches!(error, FileError::Refused(_)), "answered {error:?}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_write_family_request_against_a_kb_source_is_refused() {
    let base = TempDir::new().unwrap();
    let source = folder_source("", base.path());
    let (client, mailbox, _hub) = harness();
    let files = Files::start();
    files.submit(files::Job {
        kind: files::JobKind::Kb {
            project_id: ProjectId::generate(),
            source,
            base: base.path().to_path_buf(),
            request: files::Request::Write {
                rel_path: "new.md".to_string(),
                bytes: b"nope".to_vec(),
                expected: None,
                overwrite: false,
            },
        },
        reply_to: mailbox,
    });

    let message = client.from_host().recv_timeout(PATIENCE).unwrap();
    match message {
        Message::KbFileError { error, .. } => {
            assert!(matches!(error, FileError::Refused(_)), "answered {error:?}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(!base.path().join("new.md").exists());
}

// ── a source's derived state ──────────────────────────────────────────

#[test]
fn a_folder_source_naming_a_missing_directory_is_failed() {
    let config = TempDir::new().unwrap();
    let missing = TempDir::new().unwrap();
    let missing_path = missing.path().to_path_buf();
    drop(missing); // the directory is gone, but the path is still what the source names

    let (_client, mailbox, _hub) = harness();
    let kb = Kb::new(config.path().to_path_buf());
    let project = ProjectId::generate();
    let source = folder_source("", &missing_path);

    let statuses = kb.set_sources(project, vec![source], mailbox, no_project_path());
    assert!(matches!(statuses[0].state, KbSourceState::Failed { .. }));
}

#[test]
fn a_git_source_s_base_path_is_under_the_project_s_own_kb_area() {
    // The one place a git source's directory is composed — proof that it never lands inside the
    // project the interface picked, and that two sources never collide on the same folder.
    let config = TempDir::new().unwrap();
    let kb = Kb::new(config.path().to_path_buf());
    let project = ProjectId::generate();
    let source = KbSource {
        id: KbSourceId::generate(),
        name: "wiki".to_string(),
        origin: KbOrigin::Git {
            url: "https://example.invalid/nothing.git".to_string(),
            branch: None,
            store: Default::default(),
        },
        filter: String::new(),
        access: Default::default(),
    };

    let base = kb.base_path(project, &source, no_project_path());
    assert!(base.starts_with(config.path()));
    assert!(base.ends_with(source.id.to_string()));
    assert!(!base.exists(), "nothing has cloned into it yet");
}

#[test]
fn an_internal_source_is_ready_immediately_and_lives_under_the_project_s_wiki_area() {
    let config = TempDir::new().unwrap();
    let kb = Kb::new(config.path().to_path_buf());
    let project = ProjectId::generate();
    let source = KbSource {
        id: KbSourceId::generate(),
        name: "Wiki".to_string(),
        origin: KbOrigin::Internal,
        filter: String::new(),
        access: Default::default(),
    };

    let (_client, mailbox, _hub) = harness();
    let statuses = kb.set_sources(project, vec![source], mailbox, no_project_path());
    assert_eq!(statuses[0].state, KbSourceState::Ready);

    let base = kb.base_path(project, &statuses[0].source, no_project_path());
    assert!(base.starts_with(config.path()));
    assert!(base.ends_with("wiki"));
    assert!(base.is_dir(), "the wiki directory is created on demand");
}

// ── writing a source, the way the coordinator resolves it ─────────────
//
// `ops` is a free-function module, tested on its own in `kb::ops::tests` for the shape of every
// refusal; what is worth proving here is that `Kb::find` and `Kb::base_path` — the two calls the
// coordinator makes before it ever reaches `ops` — hand it exactly what it needs, for a source
// configured the ordinary way rather than built by hand.

#[test]
fn a_write_to_a_read_only_source_found_through_kb_is_refused() {
    let config = TempDir::new().unwrap();
    let folder = TempDir::new().unwrap();
    let (_client, mailbox, _hub) = harness();
    let project = ProjectId::generate();
    let mut source = folder_source("", folder.path());
    source.access = KbAccess::ReadOnly;

    let kb = Kb::new(config.path().to_path_buf());
    kb.set_sources(project, vec![source.clone()], mailbox, no_project_path());

    let found = kb.find(project, source.id).unwrap();
    let base = kb.base_path(project, &found, no_project_path());
    let error = ops::write_file(&base, &found, "notes.md", "hi").unwrap_err();
    assert!(matches!(error, FileError::Refused(_)));
    assert!(!folder.path().join("notes.md").exists());
}

#[test]
fn a_write_to_a_readwrite_folder_source_found_through_kb_lands_on_disk() {
    let config = TempDir::new().unwrap();
    let folder = TempDir::new().unwrap();
    let (_client, mailbox, _hub) = harness();
    let project = ProjectId::generate();
    let mut source = folder_source("", folder.path());
    source.access = KbAccess::ReadWrite;

    let kb = Kb::new(config.path().to_path_buf());
    kb.set_sources(project, vec![source.clone()], mailbox, no_project_path());

    let found = kb.find(project, source.id).unwrap();
    let base = kb.base_path(project, &found, no_project_path());
    ops::write_file(&base, &found, "notes.md", "hi").unwrap();
    assert_eq!(
        fs::read_to_string(folder.path().join("notes.md")).unwrap(),
        "hi"
    );
}
