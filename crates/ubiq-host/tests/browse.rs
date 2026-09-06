//! Listing an absolute path on the host, with no project in scope — the host-browse family's
//! worker logic, against a real directory.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use tempfile::TempDir;
use ubiq_host::files::browse;
use ubiq_proto::files::{EntryKind, HostPathError};

/// A folder holding two directories and a file, one of the directories a dotfile.
fn scratch() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("b.txt"), b"b\n").unwrap();
    fs::create_dir(dir.path().join("a_dir")).unwrap();
    fs::create_dir(dir.path().join(".hidden")).unwrap();
    dir
}

#[test]
fn a_listing_puts_directories_first_then_names_without_case() {
    let dir = scratch();
    let listing = browse::list(Some(dir.path().to_str().unwrap())).unwrap();

    let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    // Both directories (".hidden" and "a_dir") sort before the file, case-insensitively.
    assert_eq!(names, vec![".hidden", "a_dir", "b.txt"]);
    assert_eq!(listing.entries[0].kind, EntryKind::Dir);
    assert_eq!(listing.entries[1].kind, EntryKind::Dir);
    assert_eq!(listing.entries[2].kind, EntryKind::File);
}

#[test]
fn a_dotfile_is_marked_hidden_rather_than_omitted() {
    let dir = scratch();
    let listing = browse::list(Some(dir.path().to_str().unwrap())).unwrap();

    let hidden = listing
        .entries
        .iter()
        .find(|e| e.name == ".hidden")
        .unwrap();
    assert!(hidden.hidden);
    let visible = listing.entries.iter().find(|e| e.name == "b.txt").unwrap();
    assert!(!visible.hidden);
    // Marked, not dropped: the entry is still in the listing.
    assert_eq!(listing.entries.len(), 3);
}

#[test]
fn a_readable_directory_and_file_are_reported_as_readable() {
    let dir = scratch();
    let listing = browse::list(Some(dir.path().to_str().unwrap())).unwrap();
    for entry in &listing.entries {
        assert!(entry.readable, "{} should be readable", entry.name);
    }
}

#[test]
fn a_nonexistent_path_answers_with_the_failure_variant() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("nope");
    let error = browse::list(Some(missing.to_str().unwrap())).unwrap_err();
    assert_eq!(error, HostPathError::Missing);
}

#[test]
fn a_file_rather_than_a_directory_answers_with_the_failure_variant() {
    let dir = scratch();
    let file = dir.path().join("b.txt");
    let error = browse::list(Some(file.to_str().unwrap())).unwrap_err();
    assert_eq!(error, HostPathError::NotADirectory);
}

#[test]
fn the_parent_is_none_at_the_filesystem_root() {
    let listing = browse::list(Some("/")).unwrap();
    assert_eq!(listing.path, "/");
    assert_eq!(listing.parent, None);
}

#[test]
fn the_parent_is_some_below_the_root() {
    let dir = scratch();
    let listing = browse::list(Some(dir.path().to_str().unwrap())).unwrap();
    assert!(listing.parent.is_some());
}

#[test]
fn an_unreadable_directory_is_reported_rather_than_crashing() {
    let dir = scratch();
    let locked = dir.path().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

    // A user no permission bit applies to — root — reads it anyway, and has nothing to say about
    // the rule. Probing is how that is known without asking who we are.
    let enforced = fs::read_dir(&locked).is_err();
    let answer = browse::list(Some(locked.to_str().unwrap()));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    if enforced {
        let error = answer.unwrap_err();
        assert!(matches!(error, HostPathError::Denied(_)), "answered {error:?}");
    }
}

#[test]
fn an_unreadable_directory_entry_is_marked_unreadable_rather_than_crashing() {
    let dir = scratch();
    let locked = dir.path().join("locked_child");
    fs::create_dir(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

    let enforced = fs::read_dir(&locked).is_err();
    let listing = browse::list(Some(dir.path().to_str().unwrap()));

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    let listing = listing.unwrap();
    if enforced {
        let entry = listing
            .entries
            .iter()
            .find(|e| e.name == "locked_child")
            .unwrap();
        assert!(!entry.readable);
    }
}

#[test]
fn the_default_request_answers_with_a_real_absolute_path() {
    let listing = browse::list(None).unwrap();
    assert!(listing.path.starts_with('/'), "{}", listing.path);
    assert!(std::path::Path::new(&listing.path).is_dir());
}
