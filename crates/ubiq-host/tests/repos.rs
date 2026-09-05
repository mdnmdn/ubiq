//! The two roots a clone writes to, and the gate that decides whether Ubiq may delete a folder.
//!
//! Nothing here touches a network: what is worth testing about cloning is not the transfer, it is
//! the deletion. `temporary` is set for a folder the user dragged in from anywhere on their disk
//! *and* for an ephemeral clone Ubiq made, so the flag cannot be what authorises a removal — where
//! the folder sits is. The drag-drop case below is the one that matters.

use std::fs;
use std::path::Path;

use chrono::{TimeZone, Utc};
use tempfile::TempDir;
use ubiq_host::projects::Projects;
use ubiq_host::settings::{ephemeral_root, projects_root};
use ubiq_host::store::memory::{MemoryPreferenceStore, MemoryProjectStore};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::ProjectRecord;
use ubiq_proto::settings::HostSettings;

fn record(id: ProjectId, path: &Path, temporary: bool) -> ProjectRecord {
    ProjectRecord {
        id,
        name: "cloned".to_string(),
        path: path.to_string_lossy().into_owned(),
        colour: 0,
        custom_colour: None,
        temporary,
        created_at: Utc.with_ymd_and_hms(2026, 9, 5, 9, 0, 0).unwrap(),
        last_opened_at: None,
        search_excludes: Vec::new(),
        no_local_index: false,
    }
}

/// A catalogue holding `records`, with its ephemeral gate pointed at `ephemeral`.
fn catalogue(root: &Path, ephemeral: &Path, records: Vec<ProjectRecord>) -> Projects {
    let (mut projects, _) = Projects::open(
        root.to_path_buf(),
        Box::new(MemoryProjectStore::with(records)),
        Box::new(MemoryPreferenceStore::new()),
    );
    projects.point_ephemeral_at(ephemeral.to_path_buf());
    projects
}

#[test]
fn forgetting_deletes_a_clone_inside_the_ephemeral_root() {
    let home = TempDir::new().unwrap();
    let ephemeral = home.path().join("ephemeral");
    let folder = ephemeral.join("cloned");
    fs::create_dir_all(folder.join("src")).unwrap();

    let id = ProjectId::generate();
    let mut projects = catalogue(home.path(), &ephemeral, vec![record(id, &folder, true)]);
    projects.forget(id);

    assert!(!folder.exists(), "an ephemeral clone's folder goes with it");
}

/// The one that matters. A dropped folder is `temporary` too, and it is the user's.
#[test]
fn forgetting_never_deletes_a_temporary_folder_outside_the_ephemeral_root() {
    let home = TempDir::new().unwrap();
    let elsewhere = TempDir::new().unwrap();
    let folder = elsewhere.path().join("documents-thing");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("notes.md"), "mine").unwrap();

    let id = ProjectId::generate();
    let mut projects = catalogue(
        home.path(),
        &home.path().join("ephemeral"),
        vec![record(id, &folder, true)],
    );
    projects.forget(id);

    assert!(
        folder.join("notes.md").exists(),
        "a dropped folder is the user's, whatever the record claims about itself"
    );
}

/// The other half of the gate. A user is free to point the ephemeral setting at a tree full of
/// real projects, and the ordinary Forget action must stay a forget there rather than becoming a
/// delete — which is what the `temporary` half of the condition buys.
#[test]
fn forgetting_never_deletes_a_settled_project_inside_the_ephemeral_root() {
    let home = TempDir::new().unwrap();
    let ephemeral = home.path().join("ephemeral");
    let folder = ephemeral.join("a-real-project");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("notes.md"), "mine").unwrap();

    let id = ProjectId::generate();
    let mut projects = catalogue(home.path(), &ephemeral, vec![record(id, &folder, false)]);
    projects.forget(id);

    assert!(
        folder.join("notes.md").exists(),
        "a settled project is never deleted, wherever the ephemeral root happens to point"
    );
}

#[test]
fn a_path_that_climbs_out_of_the_ephemeral_root_does_not_pass_the_gate() {
    let home = TempDir::new().unwrap();
    let ephemeral = home.path().join("ephemeral");
    fs::create_dir_all(&ephemeral).unwrap();
    let escaped = home.path().join("elsewhere");
    fs::create_dir_all(&escaped).unwrap();

    // A record whose text starts inside the root and whose folder is not.
    let climbing = ephemeral.join("..").join("elsewhere");
    let id = ProjectId::generate();
    let mut projects = catalogue(home.path(), &ephemeral, vec![record(id, &climbing, true)]);
    projects.forget(id);

    assert!(escaped.exists(), "`..` is resolved before the prefix test");
}

#[test]
fn the_sweep_takes_the_folders_no_record_names() {
    let home = TempDir::new().unwrap();
    let ephemeral = home.path().join("ephemeral");
    let kept = ephemeral.join("kept");
    let orphan = ephemeral.join("orphan");
    fs::create_dir_all(&kept).unwrap();
    fs::create_dir_all(&orphan).unwrap();

    let id = ProjectId::generate();
    let mut projects = catalogue(home.path(), &ephemeral, vec![record(id, &kept, true)]);
    projects.sweep_ephemeral();

    assert!(kept.exists(), "a folder a record names stays");
    assert!(!orphan.exists(), "a folder no record names goes");
}

#[test]
fn a_catalogue_that_did_not_load_sweeps_nothing() {
    let home = TempDir::new().unwrap();
    let ephemeral = home.path().join("ephemeral");
    let clone = ephemeral.join("cloned");
    fs::create_dir_all(&clone).unwrap();

    let store = MemoryProjectStore::new();
    store.fail_load(true);
    let (mut projects, _) = Projects::open(
        home.path().to_path_buf(),
        Box::new(store),
        Box::new(MemoryPreferenceStore::new()),
    );
    projects.point_ephemeral_at(ephemeral.clone());
    projects.sweep_ephemeral();

    assert!(
        clone.exists(),
        "an empty catalogue from a corrupt file is not proof of an orphan"
    );
}

#[test]
fn the_roots_fall_back_to_the_config_root_and_honour_a_setting() {
    let config = Path::new("/dev/ubiq-config");
    let settings = HostSettings::default();
    assert_eq!(projects_root(&settings, config), config.join("clones"));
    assert_eq!(ephemeral_root(&settings, config), config.join("ephemeral"));

    let named = HostSettings {
        projects_root: Some("/work/clones".to_string()),
        // Blank is not a path: taking it as one would point a clone at the working directory.
        ephemeral_root: Some("  ".to_string()),
        ..HostSettings::default()
    };
    assert_eq!(projects_root(&named, config), Path::new("/work/clones"));
    assert_eq!(ephemeral_root(&named, config), config.join("ephemeral"));
}
