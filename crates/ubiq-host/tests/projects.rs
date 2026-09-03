//! The catalogue store, the health probe and the collector.
//!
//! The interesting cases are all failures — a corrupt file, an unwritable directory, a folder that
//! went away — because those are what the design actually turns on.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use chrono::{TimeZone, Utc};
use tempfile::TempDir;
use ubiq_host::gc;
use ubiq_host::health::probe;
use ubiq_host::store::file::FileProjectStore;
use ubiq_host::store::{ProjectStore, StoreError};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord};

fn record(name: &str, path: &str) -> ProjectRecord {
    ProjectRecord {
        id: ProjectId::generate(),
        name: name.to_string(),
        path: path.to_string(),
        colour: 0,
        custom_colour: None,
        temporary: false,
        created_at: Utc.with_ymd_and_hms(2026, 8, 14, 9, 12, 44).unwrap(),
        last_opened_at: None,
    }
}

fn store(dir: &TempDir) -> FileProjectStore {
    FileProjectStore::new(dir.path().join("projects.toml"))
}

#[test]
fn a_catalogue_survives_the_round_trip() {
    let dir = TempDir::new().unwrap();
    let store = store(&dir);
    let one = record("ubiq", "/dev/ubiq");
    let two = record("agent-manager", "/dev/agent-manager");

    store.upsert(&one).unwrap();
    store.upsert(&two).unwrap();

    let reread = FileProjectStore::new(dir.path().join("projects.toml"));
    let mut got = reread.load().unwrap();
    got.sort_by_key(|r| r.id);
    let mut want = vec![one, two];
    want.sort_by_key(|r| r.id);
    assert_eq!(got, want);
}

#[test]
fn an_absent_catalogue_is_an_empty_one_rather_than_a_failure() {
    let dir = TempDir::new().unwrap();
    assert!(store(&dir).load().unwrap().is_empty());
}

#[test]
fn a_rewrite_leaves_no_temporary_file_beside_it() {
    let dir = TempDir::new().unwrap();
    let store = store(&dir);
    for n in 0..5 {
        store.upsert(&record(&format!("p{n}"), "/dev/p")).unwrap();
    }

    let litter: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(litter.is_empty(), "left {litter:?} behind");
}

#[test]
fn an_update_replaces_rather_than_duplicates() {
    let dir = TempDir::new().unwrap();
    let store = store(&dir);
    let mut one = record("ubiq", "/dev/ubiq");
    store.upsert(&one).unwrap();

    one.name = "renamed".to_string();
    one.colour = 4;
    store.upsert(&one).unwrap();

    let got = store.load().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "renamed");
    assert_eq!(got[0].colour, 4);
}

#[test]
fn removing_takes_only_the_one_named() {
    let dir = TempDir::new().unwrap();
    let store = store(&dir);
    let one = record("one", "/dev/one");
    let two = record("two", "/dev/two");
    store.upsert(&one).unwrap();
    store.upsert(&two).unwrap();

    store.remove(one.id).unwrap();

    let got = store.load().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id, two.id);
}

#[test]
fn a_corrupt_catalogue_is_preserved_and_the_session_starts_empty() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("projects.toml");
    fs::write(&path, "version = 1\nthis is not a catalogue\n").unwrap();

    let error = store(&dir).load().unwrap_err();

    let preserved = match error {
        StoreError::Parse { preserved_as, .. } => preserved_as.expect("kept aside"),
        other => panic!("expected a parse failure, got {other:?}"),
    };
    assert!(preserved.exists(), "the original must still be on disk");
    assert!(
        preserved.to_string_lossy().contains(".corrupt-"),
        "and be findable: {}",
        preserved.display()
    );
    // Losing a catalogue silently is worse than starting without one loudly.
    assert!(!path.exists() || fs::read_to_string(&path).unwrap().is_empty());
}

#[test]
fn a_catalogue_from_a_newer_ubiq_is_never_overwritten() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("projects.toml");
    let body = "version = 99\n\n[[project]]\nid = \"01JD2W8YQ4T6M4S9H3B7GRC2VK\"\n";
    fs::write(&path, body).unwrap();

    let store = store(&dir);
    let error = store.load().unwrap_err();
    assert!(matches!(
        error,
        StoreError::UnknownVersion { found: 99, .. }
    ));

    // The session carries on, and every later write refuses rather than clobbering the file.
    let write = store.upsert(&record("new", "/dev/new"));
    assert!(matches!(write, Err(StoreError::NotDurable)));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        body,
        "the newer file must be exactly as it was"
    );
}

#[test]
fn an_unwritable_store_keeps_going_and_says_so_once() {
    let dir = TempDir::new().unwrap();
    // A file where the directory should be: every write into it fails.
    let blocked = dir.path().join("wall");
    fs::write(&blocked, "not a directory").unwrap();
    let store = FileProjectStore::new(blocked.join("projects.toml"));

    let first = store.upsert(&record("one", "/dev/one"));
    assert!(matches!(first, Err(StoreError::Io { .. })), "got {first:?}");

    // Mutations still apply in memory, and the second failure is the quieter one — which is how
    // the user is told once rather than on every change for the rest of the session.
    let second = store.upsert(&record("two", "/dev/two"));
    assert!(
        matches!(second, Err(StoreError::NotDurable)),
        "got {second:?}"
    );
}

// ── health ──────────────────────────────────────────────────────────

#[test]
fn health_says_what_is_actually_there() {
    let dir = TempDir::new().unwrap();
    let folder = dir.path().join("a-project");
    fs::create_dir(&folder).unwrap();
    let file = dir.path().join("a-file");
    fs::write(&file, "x").unwrap();

    assert_eq!(probe(&folder), ProjectHealth::Ok);
    assert_eq!(probe(&file), ProjectHealth::NotADirectory);
    assert_eq!(probe(&dir.path().join("nothing")), ProjectHealth::Missing);
}

#[cfg(unix)]
#[test]
fn a_symlink_is_judged_by_what_it_points_at() {
    let dir = TempDir::new().unwrap();
    let folder = dir.path().join("real");
    fs::create_dir(&folder).unwrap();

    let good = dir.path().join("good-link");
    std::os::unix::fs::symlink(&folder, &good).unwrap();
    assert_eq!(
        probe(&good),
        ProjectHealth::Ok,
        "a link to a directory is a project"
    );

    let broken = dir.path().join("broken-link");
    std::os::unix::fs::symlink(dir.path().join("gone"), &broken).unwrap();
    // Something is there and it is not a project, so Locate is the action rather than a re-probe.
    assert_eq!(probe(&broken), ProjectHealth::NotADirectory);
}

#[test]
fn a_folder_that_comes_back_probes_ok_again() {
    let dir = TempDir::new().unwrap();
    let folder = dir.path().join("mounted");
    fs::create_dir(&folder).unwrap();
    assert_eq!(probe(&folder), ProjectHealth::Ok);

    fs::remove_dir(&folder).unwrap();
    assert_eq!(probe(&folder), ProjectHealth::Missing);

    // Nothing was lost, because nothing was removed.
    fs::create_dir(&folder).unwrap();
    assert_eq!(probe(&folder), ProjectHealth::Ok);
}

// ── the collector ───────────────────────────────────────────────────

fn project_dir(root: &Path, name: &str) -> std::path::PathBuf {
    let dir = root.join("projects").join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("view.toml"), "version = 1").unwrap();
    dir
}

#[test]
fn the_collector_takes_only_directories_no_record_names() {
    let root = TempDir::new().unwrap();
    let kept = ProjectId::generate();
    let orphan = ProjectId::generate();

    let kept_dir = project_dir(root.path(), &kept.to_string());
    let orphan_dir = project_dir(root.path(), &orphan.to_string());
    // Never delete what you cannot identify: this was not put there by the catalogue.
    let stranger = project_dir(root.path(), "notes-i-left-here");

    let keep: HashSet<ProjectId> = [kept].into_iter().collect();
    assert_eq!(gc::collect(root.path(), &keep), 1);

    assert!(
        kept_dir.exists(),
        "a project still in the catalogue keeps its directory"
    );
    assert!(!orphan_dir.exists(), "one with no record is collected");
    assert!(stranger.exists(), "a name that is not an id is left alone");
}

#[test]
fn the_collector_is_quiet_when_there_is_nothing_to_collect() {
    let root = TempDir::new().unwrap();
    assert_eq!(gc::collect(root.path(), &HashSet::new()), 0);
}
