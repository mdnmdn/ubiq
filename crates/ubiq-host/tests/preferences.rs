//! View state: where it goes, and the rules that differ from the catalogue's.
//!
//! The host never reads the value. Everything here is about the envelope around it — where it
//! lands, and that a failure to store it is not an event anybody has to read.

use std::fs;

use tempfile::TempDir;
use ubiq_host::store::file::FilePreferenceStore;
use ubiq_host::store::memory::MemoryPreferenceStore;
use ubiq_host::store::{PreferenceStore, StoreError};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::Scope;

const BLOB: &str = r#"{"schema":1,"rail":"Ide","left":true}"#;

#[test]
fn each_scope_has_its_own_file() {
    let dir = TempDir::new().unwrap();
    let store = FilePreferenceStore::new(dir.path().to_path_buf());
    let id = ProjectId::generate();

    // One file per project rather than a section in the catalogue, so a panel drag never rewrites
    // the list the user may be hand-editing.
    assert_eq!(
        store.path(&Scope::Interface),
        dir.path().join("preferences.toml")
    );
    assert_eq!(
        store.path(&Scope::Project(id)),
        dir.path()
            .join("projects")
            .join(id.to_string())
            .join("view.toml")
    );
}

#[test]
fn a_blob_comes_back_exactly_as_it_went_in() {
    let dir = TempDir::new().unwrap();
    let store = FilePreferenceStore::new(dir.path().to_path_buf());
    let id = ProjectId::generate();

    store.set(&Scope::Interface, BLOB).unwrap();
    store.set(&Scope::Project(id), "a different one").unwrap();

    assert_eq!(store.get(&Scope::Interface).unwrap().as_deref(), Some(BLOB));
    assert_eq!(
        store.get(&Scope::Project(id)).unwrap().as_deref(),
        Some("a different one")
    );
}

#[test]
fn a_blob_with_toml_metacharacters_survives() {
    let dir = TempDir::new().unwrap();
    let store = FilePreferenceStore::new(dir.path().to_path_buf());
    // The value is opaque, so it has to survive being anything at all.
    let awkward = "line\nbreak \"quotes\" [brackets] = and 'single'";

    store.set(&Scope::Interface, awkward).unwrap();

    assert_eq!(
        store.get(&Scope::Interface).unwrap().as_deref(),
        Some(awkward)
    );
}

#[test]
fn never_set_is_not_the_same_as_empty() {
    let dir = TempDir::new().unwrap();
    let store = FilePreferenceStore::new(dir.path().to_path_buf());

    assert_eq!(store.get(&Scope::Interface).unwrap(), None);

    store.set(&Scope::Interface, "").unwrap();
    assert_eq!(store.get(&Scope::Interface).unwrap(), Some(String::new()));
}

#[test]
fn unreadable_view_state_is_discarded_rather_than_preserved() {
    let dir = TempDir::new().unwrap();
    let store = FilePreferenceStore::new(dir.path().to_path_buf());
    let path = store.path(&Scope::Interface);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "= not an envelope =").unwrap();

    // The window opens on defaults. The host never read the value, so it cannot say more — and
    // where a splitter sat is not worth keeping a corrupt file around for.
    assert_eq!(store.get(&Scope::Interface).unwrap(), None);
}

#[test]
fn clearing_a_scope_that_was_never_set_is_fine() {
    let dir = TempDir::new().unwrap();
    let store = FilePreferenceStore::new(dir.path().to_path_buf());

    assert!(store.clear(&Scope::Interface).is_ok());

    store.set(&Scope::Interface, BLOB).unwrap();
    store.clear(&Scope::Interface).unwrap();
    assert_eq!(store.get(&Scope::Interface).unwrap(), None);
}

#[test]
fn a_scope_is_keyed_by_the_project_it_names() {
    let store = MemoryPreferenceStore::new();
    let one = Scope::Project(ProjectId::generate());
    let two = Scope::Project(ProjectId::generate());

    store.set(&one, "first").unwrap();
    store.set(&two, "second").unwrap();

    assert_eq!(store.get(&one).unwrap().as_deref(), Some("first"));
    assert_eq!(store.get(&two).unwrap().as_deref(), Some("second"));
}

#[test]
fn a_store_that_refuses_to_write_still_answers() {
    let store = MemoryPreferenceStore::new();
    store.fail_writes(true);

    let failed = store.set(&Scope::Interface, BLOB);

    assert!(matches!(failed, Err(StoreError::NotDurable)));
    // The caller logs it and carries on: this never becomes a `ProjectError`.
    assert_eq!(store.get(&Scope::Interface).unwrap(), None);
}
