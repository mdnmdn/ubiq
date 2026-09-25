//! The catalogue store, the health probe and the collector.
//!
//! The interesting cases are all failures — a corrupt file, an unwritable directory, a folder that
//! went away — because those are what the design actually turns on.

#[cfg(feature = "harness")]
use std::collections::HashSet;
use std::fs;
#[cfg(feature = "harness")]
use std::path::Path;

use chrono::{TimeZone, Utc};
use tempfile::TempDir;
#[cfg(feature = "harness")]
use ubiq_host::gc;
use ubiq_host::health::probe;
use ubiq_host::projects::Projects;
use ubiq_host::store::file::{FileProjectStore, FileTaskStore};
use ubiq_host::store::memory::{MemoryPreferenceStore, MemoryProjectStore};
use ubiq_host::store::project_dir;
use ubiq_host::store::{ProjectStore, StoreError};
use ubiq_proto::ids::{ProjectId, SshProfileId};
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{DroneChange, DroneOrigin, ProjectHealth, ProjectRecord, StorageMode};
use ubiq_proto::settings::DronePreset;

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
        search_excludes: Vec::new(),
        index: None,
        mission_term: None,
        managed_repos: Vec::new(),
        tools: Vec::new(),
        lanes: Vec::new(),
        runs_on: None,
        initials: String::new(),
        storage: StorageMode::UbiqManaged,
    }
}

/// A drone origin worth telling apart from "local": a real profile, a real far root.
fn origin() -> DroneOrigin {
    DroneOrigin {
        profile: SshProfileId::generate(),
        root: "/far/root".to_string(),
        preset: DronePreset::Session,
        linger_secs: None,
    }
}

/// A catalogue holding one record, in memory, for the `runs_on` tests below. The `TempDir` is
/// returned alongside so its folder outlives the catalogue that reserves a workarea inside it.
fn projects_with(record: ProjectRecord) -> (Projects, ProjectId, TempDir) {
    let id = record.id;
    let dir = TempDir::new().unwrap();
    let (projects, _) = Projects::open(
        dir.path().to_path_buf(),
        Box::new(MemoryProjectStore::with(vec![record])),
        Box::new(MemoryPreferenceStore::new()),
    );
    (projects, id, dir)
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

/// `Projects::add` is the one point every creation path goes through — the "Add project" dialog
/// passes a colour it picked itself, but drag-and-drop and others pass `None`. Without a host-side
/// fallback, every such project lands on swatch zero and the Teams filter shows one dot for all of
/// them (T-197). Adding a second project with no colour, next to one already on swatch zero, must
/// not repeat it.
#[test]
fn adding_with_no_colour_picks_one_not_already_in_use() {
    let dir = TempDir::new().unwrap();
    let existing_folder = dir.path().join("existing");
    let new_folder = dir.path().join("new");
    fs::create_dir_all(&existing_folder).unwrap();
    fs::create_dir_all(&new_folder).unwrap();

    let mut existing = record("existing", &existing_folder.to_string_lossy());
    existing.colour = 0;
    let (mut projects, _) = Projects::open(
        dir.path().to_path_buf(),
        Box::new(MemoryProjectStore::with(vec![existing])),
        Box::new(MemoryPreferenceStore::new()),
    );

    let replies = projects.add(
        &new_folder.to_string_lossy(),
        Some("new".to_string()),
        None,
        None,
        false,
        StorageMode::UbiqManaged,
    );

    let colour = replies
        .into_iter()
        .find_map(|reply| match reply.into_message() {
            Message::ProjectAdded { project } => Some(project.record.colour),
            _ => None,
        })
        .expect("the project was added");
    assert_ne!(
        colour, 0,
        "landed on the same swatch as the existing project"
    );
}

/// Once every swatch is in use, the fewest-used rule wraps rather than refusing — the same as the
/// interface's `AppState::next_colour`, which this mirrors.
#[test]
fn adding_with_no_colour_wraps_once_the_palette_is_exhausted() {
    let dir = TempDir::new().unwrap();
    let mut records = Vec::new();
    for index in 0..16usize {
        let folder = dir.path().join(format!("p{index}"));
        fs::create_dir_all(&folder).unwrap();
        let mut r = record(&format!("p{index}"), &folder.to_string_lossy());
        r.colour = index;
        records.push(r);
    }
    let (mut projects, _) = Projects::open(
        dir.path().to_path_buf(),
        Box::new(MemoryProjectStore::with(records)),
        Box::new(MemoryPreferenceStore::new()),
    );

    let new_folder = dir.path().join("wraps");
    fs::create_dir_all(&new_folder).unwrap();
    let replies = projects.add(
        &new_folder.to_string_lossy(),
        None,
        None,
        None,
        false,
        StorageMode::UbiqManaged,
    );

    let colour = replies
        .into_iter()
        .find_map(|reply| match reply.into_message() {
            Message::ProjectAdded { project } => Some(project.record.colour),
            _ => None,
        })
        .expect("the project was added");
    // Every swatch is used exactly once, so the tie-break (lowest index) picks swatch 0 again.
    assert_eq!(colour, 0);
}

// ── runs_on ─────────────────────────────────────────────────────────

#[test]
fn updating_a_project_sets_its_drone_origin() {
    let (mut projects, id, _dir) = projects_with(record("ubiq", "/dev/ubiq"));
    let origin = origin();

    projects.update(
        id,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(DroneChange::Set(origin.clone())),
    );

    assert_eq!(projects.record(id).unwrap().runs_on, Some(origin));
}

#[test]
fn an_update_that_says_nothing_about_the_drone_keeps_it() {
    let (mut projects, id, _dir) = projects_with(record("ubiq", "/dev/ubiq"));
    let origin = origin();
    projects.update(
        id,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(DroneChange::Set(origin.clone())),
    );

    // A rename says nothing about where the project runs, so it keeps what it had.
    projects.update(
        id,
        Some("renamed".to_string()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );

    let after = projects.record(id).unwrap();
    assert_eq!(after.name, "renamed");
    assert_eq!(after.runs_on, Some(origin));
}

#[test]
fn updating_a_project_can_bring_it_back_home() {
    let (mut projects, id, _dir) = projects_with(record("ubiq", "/dev/ubiq"));
    projects.update(
        id,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(DroneChange::Set(origin())),
    );

    projects.update(
        id,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(DroneChange::Local),
    );

    assert_eq!(projects.record(id).unwrap().runs_on, None);
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

#[cfg(feature = "harness")]
fn project_dir(root: &Path, name: &str) -> std::path::PathBuf {
    let dir = root.join("projects").join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("view.toml"), "version = 1").unwrap();
    dir
}

// The collector is the catalogue's one edge into the harness library, so these two go with it.
#[cfg(feature = "harness")]
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

#[cfg(feature = "harness")]
#[test]
fn the_collector_is_quiet_when_there_is_nothing_to_collect() {
    let root = TempDir::new().unwrap();
    assert_eq!(gc::collect(root.path(), &HashSet::new()), 0);
}

// ── project-managed storage ─────────────────────────────────────────

/// The whole of what creating a project-managed project leaves on disk: the folder, the ignore
/// file, the metadata, and the tasks landing inside the project rather than under the config root.
#[test]
fn a_project_managed_project_keeps_its_data_in_its_own_folder() {
    let config = TempDir::new().unwrap();
    let folder = TempDir::new().unwrap();
    let (mut projects, _) = Projects::open(
        config.path().to_path_buf(),
        Box::new(MemoryProjectStore::with(Vec::new())),
        Box::new(MemoryPreferenceStore::new()),
    );

    let replies = projects.add(
        &folder.path().to_string_lossy(),
        Some("shared".to_string()),
        None,
        None,
        false,
        StorageMode::ProjectManaged,
    );
    let added = replies
        .into_iter()
        .find_map(|reply| match reply.into_message() {
            Message::ProjectAdded { project } => Some(project),
            _ => None,
        })
        .expect("the project was added");
    assert_eq!(added.record.storage, StorageMode::ProjectManaged);

    let in_project = folder.path().join(".ubiq");
    assert!(in_project.join(".gitignore").is_file());
    assert_eq!(
        project_dir::read_metadata(&in_project)
            .expect("the in-project metadata")
            .name,
        "shared"
    );

    // The store resolves through the pointer, with no catalogue of its own to read.
    let tasks = FileTaskStore::new(config.path().to_path_buf());
    assert_eq!(
        tasks.path(added.record.id),
        in_project.join("tasks.toml"),
        "a project-managed project's tasks belong to the project"
    );
}

/// A Ubiq-managed project is `D30` unchanged: nothing at all inside the user's folder.
#[test]
fn a_ubiq_managed_project_writes_nothing_inside_the_project() {
    let config = TempDir::new().unwrap();
    let folder = TempDir::new().unwrap();
    let (mut projects, _) = Projects::open(
        config.path().to_path_buf(),
        Box::new(MemoryProjectStore::with(Vec::new())),
        Box::new(MemoryPreferenceStore::new()),
    );

    projects.add(
        &folder.path().to_string_lossy(),
        Some("private".to_string()),
        None,
        None,
        false,
        StorageMode::UbiqManaged,
    );
    assert!(!folder.path().join(".ubiq").exists());
}

/// A rename reaches the project's own copy, not only the catalogue.
#[test]
fn renaming_a_project_managed_project_rewrites_its_in_project_metadata() {
    let config = TempDir::new().unwrap();
    let folder = TempDir::new().unwrap();
    let (mut projects, _) = Projects::open(
        config.path().to_path_buf(),
        Box::new(MemoryProjectStore::with(Vec::new())),
        Box::new(MemoryPreferenceStore::new()),
    );
    let replies = projects.add(
        &folder.path().to_string_lossy(),
        Some("before".to_string()),
        None,
        None,
        false,
        StorageMode::ProjectManaged,
    );
    let id = replies
        .into_iter()
        .find_map(|reply| match reply.into_message() {
            Message::ProjectAdded { project } => Some(project.record.id),
            _ => None,
        })
        .expect("the project was added");

    projects.update(
        id,
        Some("after".to_string()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );

    assert_eq!(
        project_dir::read_metadata(&folder.path().join(".ubiq"))
            .expect("the in-project metadata")
            .name,
        "after"
    );
}

/// The catalogue is a lookup, and the folder is the authority: a `project.toml` that travelled
/// with the project corrects the name this machine wrote down.
#[test]
fn the_in_project_name_corrects_the_catalogue_on_load() {
    let config = TempDir::new().unwrap();
    let folder = TempDir::new().unwrap();

    let mut stale = record("stale", &folder.path().to_string_lossy());
    stale.storage = StorageMode::ProjectManaged;
    let id = stale.id;
    let mut fresh = stale.clone();
    fresh.name = "renamed elsewhere".to_string();
    project_dir::provision(config.path(), &fresh).expect("the folder is provisioned");

    let catalogue = MemoryProjectStore::with(vec![stale]);
    let (projects, _) = Projects::open(
        config.path().to_path_buf(),
        Box::new(catalogue),
        Box::new(MemoryPreferenceStore::new()),
    );

    assert_eq!(
        projects.record(id).expect("the record").name,
        "renamed elsewhere"
    );
}

/// A Ubiq-managed project never consults a folder it does not write to, so a stray `project.toml`
/// left behind by an earlier life changes nothing.
#[test]
fn a_ubiq_managed_project_ignores_an_in_project_name() {
    let config = TempDir::new().unwrap();
    let folder = TempDir::new().unwrap();

    let in_project = folder.path().join(".ubiq");
    fs::create_dir_all(&in_project).unwrap();
    let elsewhere = record("what the folder says", &folder.path().to_string_lossy());
    project_dir::write_metadata(&in_project, &elsewhere).expect("a stray copy");

    let mut kept = elsewhere.clone();
    kept.name = "what the catalogue says".to_string();
    let id = kept.id;
    let (projects, _) = Projects::open(
        config.path().to_path_buf(),
        Box::new(MemoryProjectStore::with(vec![kept])),
        Box::new(MemoryPreferenceStore::new()),
    );

    assert_eq!(
        projects.record(id).expect("the record").name,
        "what the catalogue says"
    );
}
