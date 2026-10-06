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
        protected: false,
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
            key: None,
            request: files::Request::Tree {
                rel_path: String::new(),
                depth: 1,
                prefetch: false,
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
            key: None,
            request: files::Request::Tree {
                rel_path: String::new(),
                depth: 1,
                prefetch: false,
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
            key: None,
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
            key: None,
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
        protected: false,
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
        protected: false,
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
    let error = ops::write_file(&base, &found, "notes.md", "hi", None).unwrap_err();
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
    ops::write_file(&base, &found, "notes.md", "hi", None).unwrap();
    assert_eq!(
        fs::read_to_string(folder.path().join("notes.md")).unwrap(),
        "hi"
    );
}

// ── a password-protected wiki (`D206`) ──────────────────────────────

use std::sync::Arc;
use ubiq_host::kb::vault::{KdfParams, MemoryKeychain};

fn protected_kb(config: &Path, keychain: Arc<MemoryKeychain>) -> Kb {
    Kb::with_keychain(config.to_path_buf(), keychain, KdfParams::insecure_fast())
}

fn protected_wiki() -> KbSource {
    KbSource {
        id: KbSourceId::generate(),
        name: "Vault".to_string(),
        origin: KbOrigin::Internal,
        filter: String::new(),
        access: Default::default(),
        protected: true,
    }
}

/// One read through the worker, the way the coordinator's `kb_job` hands it over.
fn read_through_worker(
    source: &KbSource,
    base: &Path,
    key: Option<Arc<ubiq_host::kb::vault::Key>>,
) -> Message {
    let (client, mailbox, _hub) = harness();
    let files = Files::start();
    files.submit(files::Job {
        kind: files::JobKind::Kb {
            project_id: ProjectId::generate(),
            source: source.clone(),
            base: base.to_path_buf(),
            request: files::Request::Read {
                rel_path: "notes.md".to_string(),
                max_bytes: None,
            },
            key,
        },
        reply_to: mailbox,
    });
    client.from_host().recv_timeout(PATIENCE).unwrap()
}

#[test]
fn a_protected_wiki_is_locked_until_unlocked_and_never_plaintext_on_disk() {
    let config = TempDir::new().unwrap();
    let kb = protected_kb(config.path(), Arc::new(MemoryKeychain::default()));
    let project = ProjectId::generate();
    let wiki = protected_wiki();
    let (_client, mailbox, _hub) = harness();

    let statuses = kb.set_sources(project, vec![wiki.clone()], mailbox, no_project_path());
    assert_eq!(statuses[0].state, KbSourceState::Locked { first_use: true });
    assert!(matches!(
        kb.vault_key(project, &wiki),
        Err(FileError::Refused(_))
    ));

    // The first unlock sets the password.
    kb.unlock(project, wiki.id, "correct horse", false).unwrap();
    assert_eq!(
        kb.sources(project, no_project_path())[0].state,
        KbSourceState::Ready
    );

    let key = kb.vault_key(project, &wiki).unwrap().expect("a key");
    let base = kb.base_path(project, &wiki, no_project_path());
    assert!(base.starts_with(config.path()) && base.ends_with("pages"));
    assert!(
        !base.ends_with("wiki"),
        "a protected wiki has a directory of its own"
    );
    ops::write_file(&base, &wiki, "notes.md", "the needle", Some(&key)).unwrap();
    let on_disk = fs::read(base.join("notes.md")).unwrap();
    assert!(!on_disk.windows(6).any(|w| w == b"needle"));

    // Read back through the worker with the key: plaintext, in memory only.
    let Message::KbFileContents { contents, .. } =
        read_through_worker(&wiki, &base, Some(key.clone()))
    else {
        panic!("expected contents")
    };
    assert_eq!(contents.bytes, b"the needle");
    assert_eq!(contents.len, 10);

    // Without the key the worker refuses — and a write without it is refused before the disk.
    let Message::KbFileError { error, .. } = read_through_worker(&wiki, &base, None) else {
        panic!("expected a refusal")
    };
    assert!(matches!(error, FileError::Refused(_)));
    assert!(matches!(
        ops::write_file(&base, &wiki, "notes.md", "x", None),
        Err(FileError::Refused(_))
    ));

    // A fresh host over the same root, no keychain entry: locked again, and a wrong password
    // stays locked.
    let again = protected_kb(config.path(), Arc::new(MemoryKeychain::default()));
    again.auto_unlock(project);
    assert_eq!(
        again.sources(project, no_project_path())[0].state,
        KbSourceState::Locked { first_use: false }
    );
    assert_eq!(
        again.unlock(project, wiki.id, "wrong", false).unwrap_err(),
        "the password is wrong"
    );
    assert!(again.vault_key(project, &wiki).is_err());
    again
        .unlock(project, wiki.id, "correct horse", false)
        .unwrap();
}

#[test]
fn a_lost_header_is_not_a_first_use_and_takes_no_new_password() {
    let config = TempDir::new().unwrap();
    let kb = protected_kb(config.path(), Arc::new(MemoryKeychain::default()));
    let project = ProjectId::generate();
    let wiki = protected_wiki();
    let (_client, mailbox, _hub) = harness();
    kb.set_sources(project, vec![wiki.clone()], mailbox, no_project_path());
    kb.unlock(project, wiki.id, "pw", false).unwrap();
    let key = kb.vault_key(project, &wiki).unwrap().expect("a key");
    let base = kb.base_path(project, &wiki, no_project_path());
    ops::write_file(&base, &wiki, "notes.md", "sealed", Some(&key)).unwrap();
    fs::remove_file(base.parent().unwrap().join(ubiq_host::kb::vault::HEADER)).unwrap();

    let again = protected_kb(config.path(), Arc::new(MemoryKeychain::default()));
    assert_eq!(
        again.sources(project, no_project_path())[0].state,
        KbSourceState::Locked { first_use: false }
    );
    assert_eq!(
        again.unlock(project, wiki.id, "new", false).unwrap_err(),
        "this wiki's header is missing; its pages cannot be opened"
    );
    assert!(again.vault_key(project, &wiki).is_err());
}

#[test]
fn a_remembered_password_opens_the_wiki_on_the_next_run() {
    let config = TempDir::new().unwrap();
    let keychain = Arc::new(MemoryKeychain::default());
    let project = ProjectId::generate();
    let wiki = protected_wiki();
    let (_client, mailbox, _hub) = harness();

    let kb = protected_kb(config.path(), keychain.clone());
    kb.set_sources(project, vec![wiki.clone()], mailbox, no_project_path());
    kb.unlock(project, wiki.id, "pw", true).unwrap();
    assert!(keychain.holds(wiki.id));

    let next_run = protected_kb(config.path(), keychain.clone());
    next_run.auto_unlock(project);
    assert_eq!(
        next_run.sources(project, no_project_path())[0].state,
        KbSourceState::Ready
    );

    // Unlocking without `remember` forgets what was filed.
    next_run.unlock(project, wiki.id, "pw", false).unwrap();
    assert!(!keychain.holds(wiki.id));
}

#[test]
fn changing_the_password_re_encrypts_and_retires_the_old_one() {
    let config = TempDir::new().unwrap();
    let keychain = Arc::new(MemoryKeychain::default());
    let project = ProjectId::generate();
    let wiki = protected_wiki();
    let (_client, mailbox, _hub) = harness();

    let kb = protected_kb(config.path(), keychain.clone());
    kb.set_sources(project, vec![wiki.clone()], mailbox, no_project_path());
    kb.unlock(project, wiki.id, "old", false).unwrap();
    let base = kb.base_path(project, &wiki, no_project_path());
    let old_key = kb.vault_key(project, &wiki).unwrap().unwrap();
    ops::write_file(&base, &wiki, "notes.md", "kept", Some(&old_key)).unwrap();
    let before = fs::read(base.join("notes.md")).unwrap();

    assert!(
        kb.change_password(project, wiki.id, "nope", "new", true)
            .is_err()
    );
    assert_eq!(fs::read(base.join("notes.md")).unwrap(), before);

    kb.change_password(project, wiki.id, "old", "new", true)
        .unwrap();
    assert!(keychain.holds(wiki.id));
    assert_ne!(fs::read(base.join("notes.md")).unwrap(), before);

    let fresh = protected_kb(config.path(), Arc::new(MemoryKeychain::default()));
    assert!(fresh.unlock(project, wiki.id, "old", false).is_err());
    fresh.unlock(project, wiki.id, "new", false).unwrap();
    let key = fresh.vault_key(project, &wiki).unwrap();
    let Message::KbFileContents { contents, .. } = read_through_worker(&wiki, &base, key) else {
        panic!("expected contents")
    };
    assert_eq!(contents.bytes, b"kept");
}

#[test]
fn protection_is_fixed_at_creation_and_only_for_a_wiki() {
    let config = TempDir::new().unwrap();
    let folder = TempDir::new().unwrap();
    let keychain = Arc::new(MemoryKeychain::default());
    let kb = protected_kb(config.path(), keychain.clone());
    let project = ProjectId::generate();
    let wiki = protected_wiki();
    let mut docs = folder_source("", folder.path());
    docs.protected = true;
    let (_client, mailbox, _hub) = harness();

    let statuses = kb.set_sources(
        project,
        vec![wiki.clone(), docs.clone()],
        mailbox.clone(),
        no_project_path(),
    );
    assert!(
        !statuses[1].source.protected,
        "only a wiki may be protected"
    );

    // Saving the list again with the flag cleared does not unprotect it.
    let mut unflagged = wiki.clone();
    unflagged.protected = false;
    let statuses = kb.set_sources(project, vec![unflagged], mailbox.clone(), no_project_path());
    assert!(statuses[0].source.protected);

    // Removing it forgets its filed password.
    kb.unlock(project, wiki.id, "pw", true).unwrap();
    assert!(keychain.holds(wiki.id));
    kb.set_sources(project, vec![], mailbox, no_project_path());
    assert!(!keychain.holds(wiki.id));
}
