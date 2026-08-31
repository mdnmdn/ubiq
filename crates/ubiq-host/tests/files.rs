//! A project's tree and its files, against a real directory.
//!
//! The path tests come first and are the point of the file: every one of them is a way out of the
//! project's root, and the refusals are what stop the interface — or anything that learns to speak
//! the contract — from reading or writing somewhere the user never named.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use tempfile::TempDir;
use ubiq_host::files::{self, path};
use ubiq_proto::files::{EntryKind, FileError, FileVersion};

/// A project holding a file and a folder with a file in it.
fn project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("top.txt"), b"top\n").unwrap();
    fs::create_dir(dir.path().join("sub")).unwrap();
    fs::write(dir.path().join("sub/inner.txt"), b"inner\n").unwrap();
    dir
}

fn refused(error: &FileError) -> bool {
    matches!(error, FileError::Refused(_))
}

// ── the boundary ────────────────────────────────────────────────────

#[test]
fn a_parent_component_is_refused_rather_than_popped() {
    let dir = project();
    let root = dir.path();

    for rel in ["..", "../x", "sub/../../b", "sub/../..", "a/b/../../.."] {
        let error = path::resolve(root, rel).expect_err("{rel} should be refused");
        assert!(refused(&error), "{rel} answered {error:?}");
    }

    // `./` says nothing and is dropped rather than refused.
    assert_eq!(
        path::resolve(root, "./top.txt").unwrap(),
        fs::canonicalize(root.join("top.txt")).unwrap()
    );
}

#[test]
fn an_absolute_rel_path_is_refused() {
    let dir = project();

    for rel in ["/etc/passwd", "/", "/tmp"] {
        let error = path::resolve(dir.path(), rel).expect_err("{rel} should be refused");
        assert!(refused(&error), "{rel} answered {error:?}");
    }
}

#[test]
fn a_symlink_out_of_the_root_is_listed_and_never_followed() {
    let dir = project();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("secret"), b"not yours\n").unwrap();
    fs::create_dir(outside.path().join("elsewhere")).unwrap();

    std::os::unix::fs::symlink(outside.path().join("secret"), dir.path().join("to-file")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("elsewhere"), dir.path().join("to-dir"))
        .unwrap();

    // Drawn, because a row the interface never sees is a tree that lies…
    let listing = &files::listing(dir.path(), "", 1).unwrap()[0];
    for name in ["to-file", "to-dir"] {
        let entry = listing.entries.iter().find(|e| e.name == name).unwrap();
        assert_eq!(entry.kind, EntryKind::Other, "{name} was {entry:?}");
        assert!(entry.symlink);
    }

    // …and refused when it is asked for.
    assert!(refused(
        &files::contents(dir.path(), "to-file", None).unwrap_err()
    ));
    assert!(refused(
        &files::listing(dir.path(), "to-dir", 1).unwrap_err()
    ));
}

#[test]
fn a_symlink_inside_the_root_is_followed() {
    let dir = project();
    std::os::unix::fs::symlink(dir.path().join("sub"), dir.path().join("link-to-sub")).unwrap();

    let listing = &files::listing(dir.path(), "", 1).unwrap()[0];
    let entry = listing
        .entries
        .iter()
        .find(|e| e.name == "link-to-sub")
        .unwrap();
    assert_eq!(
        entry.kind,
        EntryKind::Dir,
        "a link inside the root is a folder"
    );

    // The positive control: the check refuses what leaves, not everything.
    let through = &files::listing(dir.path(), "link-to-sub", 1).unwrap()[0];
    assert!(through.entries.iter().any(|e| e.name == "inner.txt"));
}

#[test]
fn a_root_that_is_itself_a_symlink_still_contains_its_own_children() {
    // This is the macOS `/tmp` → `/private/tmp` case, and it is what a root that is not
    // canonicalised on every request gets wrong: every legitimate child would fail containment.
    let real = project();
    let holder = TempDir::new().unwrap();
    let link = holder.path().join("project");
    std::os::unix::fs::symlink(real.path(), &link).unwrap();

    assert_eq!(
        path::resolve(&link, "sub/inner.txt").unwrap(),
        fs::canonicalize(real.path().join("sub/inner.txt")).unwrap()
    );
    let listing = &files::listing(&link, "", 1).unwrap()[0];
    assert!(listing.entries.iter().any(|e| e.name == "top.txt"));
}

#[test]
fn a_path_of_absurd_depth_is_refused() {
    let dir = project();
    let deep = vec!["a"; 200].join("/");
    assert!(refused(&path::resolve(dir.path(), &deep).unwrap_err()));
}

#[test]
fn no_reply_ever_carries_an_absolute_path() {
    let dir = project();
    let root = dir.path().to_string_lossy().into_owned();

    for listing in files::listing(dir.path(), "", 3).unwrap() {
        assert!(!listing.rel_path.starts_with('/'), "{listing:?}");
        assert!(!listing.rel_path.contains(&root), "{listing:?}");
        for entry in &listing.entries {
            assert!(!entry.rel_path.starts_with('/'), "{entry:?}");
            assert!(!entry.rel_path.contains(&root), "{entry:?}");
        }
    }
}

// ── listing ─────────────────────────────────────────────────────────

#[test]
fn a_listing_puts_directories_first_then_names_without_case() {
    let dir = TempDir::new().unwrap();
    for name in ["Zebra.txt", "apple.txt", "Banana.txt"] {
        fs::write(dir.path().join(name), b"x").unwrap();
    }
    for name in ["src", "Docs"] {
        fs::create_dir(dir.path().join(name)).unwrap();
    }

    let listings = files::listing(dir.path(), "", 1).unwrap();
    let names: Vec<&str> = listings[0]
        .entries
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["Docs", "src", "apple.txt", "Banana.txt", "Zebra.txt"]
    );
}

#[test]
fn the_ignore_set_bounds_a_deep_walk_and_not_an_explicit_one() {
    let dir = project();
    fs::create_dir(dir.path().join("node_modules")).unwrap();
    fs::write(dir.path().join("node_modules/left.js"), b"x").unwrap();

    // A walk does not descend into it…
    let listings = files::listing(dir.path(), "", 3).unwrap();
    assert!(
        listings.iter().all(|l| l.rel_path != "node_modules"),
        "a deep walk descended into the ignore set"
    );
    // …but the folder is still a row, because a tree with rows missing is a tree that lies.
    assert!(
        listings[0].entries.iter().any(|e| e.name == "node_modules"),
        "the folder itself must still be drawn"
    );

    // Asked for directly, it is answered in full.
    let explicit = &files::listing(dir.path(), "node_modules", 1).unwrap()[0];
    assert!(explicit.entries.iter().any(|e| e.name == "left.js"));
}

#[test]
fn a_directory_over_the_ceiling_is_truncated() {
    let dir = TempDir::new().unwrap();
    for n in 0..2_100 {
        fs::write(dir.path().join(format!("f{n:05}")), b"x").unwrap();
    }

    let listing = &files::listing(dir.path(), "", 1).unwrap()[0];
    assert!(listing.truncated, "the ceiling should have cut it short");
    assert_eq!(listing.entries.len(), 2_000);
}

#[test]
fn a_socket_is_neither_a_file_nor_read() {
    let dir = project();
    // A short path, because a Unix socket's name has a hard length limit.
    let socket = TempDir::new().unwrap();
    let bound = socket.path().join("s");
    let _listener = std::os::unix::net::UnixListener::bind(&bound).unwrap();
    fs::hard_link(&bound, dir.path().join("s")).ok();

    if !dir.path().join("s").exists() {
        // A filesystem that will not hard-link a socket has nothing to say about the rule.
        return;
    }

    let listing = &files::listing(dir.path(), "", 1).unwrap()[0];
    let entry = listing.entries.iter().find(|e| e.name == "s").unwrap();
    assert_eq!(entry.kind, EntryKind::Other);
    assert_eq!(
        files::contents(dir.path(), "s", None).unwrap_err(),
        FileError::WrongKind,
        "a socket would block the reader forever"
    );
}

#[test]
fn a_file_listed_as_a_directory_is_the_wrong_kind() {
    let dir = project();
    assert_eq!(
        files::listing(dir.path(), "top.txt", 1).unwrap_err(),
        FileError::WrongKind
    );
}

// ── reading ─────────────────────────────────────────────────────────

#[test]
fn a_file_under_the_ceiling_reads_whole_and_carries_a_version() {
    let dir = project();
    let read = files::contents(dir.path(), "sub/inner.txt", None).unwrap();

    assert_eq!(read.bytes, b"inner\n");
    assert_eq!(read.len, 6);
    assert!(!read.truncated);
    assert!(!read.is_binary);
    assert_eq!(read.version.unwrap().len, 6);
}

#[test]
fn a_file_over_the_ceiling_is_truncated_and_carries_no_version() {
    let dir = project();
    fs::write(dir.path().join("big.txt"), vec![b'a'; 5_000]).unwrap();

    let read = files::contents(dir.path(), "big.txt", Some(100)).unwrap();
    assert_eq!(read.bytes.len(), 100);
    assert_eq!(read.len, 5_000);
    assert!(read.truncated);
    // No version is what makes the buffer unsavable mechanically: a write naming none is refused
    // on a file that exists.
    assert!(read.version.is_none());
}

#[test]
fn a_nul_byte_makes_a_file_binary_and_high_utf8_does_not() {
    let dir = project();
    fs::write(dir.path().join("bin"), [b'M', b'Z', 0x00, 0x01]).unwrap();
    fs::write(dir.path().join("text"), "héllo — ok\n".as_bytes()).unwrap();

    assert!(files::contents(dir.path(), "bin", None).unwrap().is_binary);
    assert!(!files::contents(dir.path(), "text", None).unwrap().is_binary);
}

#[test]
fn a_directory_read_as_a_file_is_the_wrong_kind() {
    let dir = project();
    assert_eq!(
        files::contents(dir.path(), "sub", None).unwrap_err(),
        FileError::WrongKind
    );
}

#[test]
fn a_missing_file_is_missing() {
    let dir = project();
    assert_eq!(
        files::contents(dir.path(), "nope.txt", None).unwrap_err(),
        FileError::Missing
    );
}

#[test]
fn an_unreadable_file_is_denied() {
    let dir = project();
    let locked = dir.path().join("locked.txt");
    fs::write(&locked, b"x").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

    // A user no permission bit applies to — root — reads it anyway, and has nothing to say about
    // the rule. Probing is how that is known without asking who we are.
    let enforced = fs::File::open(&locked).is_err();
    let answer = files::contents(dir.path(), "locked.txt", None);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();

    if enforced {
        let error = answer.unwrap_err();
        assert!(matches!(error, FileError::Denied(_)), "answered {error:?}");
    }
}

// ── saving ──────────────────────────────────────────────────────────

#[test]
fn a_save_replaces_the_contents_and_answers_the_new_version() {
    let dir = project();
    let read = files::contents(dir.path(), "top.txt", None).unwrap();

    let version = files::save(dir.path(), "top.txt", b"rewritten\n", read.version).unwrap();
    assert_eq!(
        fs::read(dir.path().join("top.txt")).unwrap(),
        b"rewritten\n"
    );
    assert_eq!(version.len, 10);
}

#[test]
fn a_save_leaves_no_temporary_file_beside_it() {
    let dir = project();
    let mut version = files::contents(dir.path(), "top.txt", None)
        .unwrap()
        .version;
    for n in 0..5 {
        version =
            Some(files::save(dir.path(), "top.txt", format!("{n}\n").as_bytes(), version).unwrap());
    }

    let litter: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(litter.is_empty(), "left {litter:?} behind");
}

#[test]
fn a_save_with_a_stale_version_is_a_conflict_and_the_file_is_untouched() {
    let dir = project();
    let stale = FileVersion {
        len: 999,
        modified: None,
    };

    assert_eq!(
        files::save(dir.path(), "top.txt", b"clobbered\n", Some(stale)).unwrap_err(),
        FileError::Conflict
    );
    assert_eq!(fs::read(dir.path().join("top.txt")).unwrap(), b"top\n");
}

#[test]
fn a_save_naming_no_version_creates_a_file_and_refuses_an_existing_one() {
    let dir = project();

    files::save(dir.path(), "sub/new.txt", b"new\n", None).unwrap();
    assert_eq!(fs::read(dir.path().join("sub/new.txt")).unwrap(), b"new\n");

    // The only safe meaning of "no version" is creation; a forced overwrite is not on offer.
    assert_eq!(
        files::save(dir.path(), "top.txt", b"clobbered\n", None).unwrap_err(),
        FileError::Conflict
    );
    assert_eq!(fs::read(dir.path().join("top.txt")).unwrap(), b"top\n");
}

#[test]
fn a_save_onto_a_file_that_went_away_is_missing_rather_than_a_resurrection() {
    let dir = project();
    let read = files::contents(dir.path(), "top.txt", None).unwrap();
    fs::remove_file(dir.path().join("top.txt")).unwrap();

    assert_eq!(
        files::save(dir.path(), "top.txt", b"back\n", read.version).unwrap_err(),
        FileError::Missing
    );
    assert!(!dir.path().join("top.txt").exists());
}

#[test]
fn a_save_keeps_the_file_executable() {
    let dir = project();
    let script = dir.path().join("run.sh");
    fs::write(&script, b"#!/bin/sh\ntrue\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    let read = files::contents(dir.path(), "run.sh", None).unwrap();
    files::save(dir.path(), "run.sh", b"#!/bin/sh\nfalse\n", read.version).unwrap();

    let mode = fs::metadata(&script).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o755,
        "the rename replaced the inode and lost the mode"
    );
}

#[test]
fn a_save_never_writes_through_a_symlink_out_of_the_root() {
    let dir = project();
    let outside = TempDir::new().unwrap();
    let victim = outside.path().join("victim");
    fs::write(&victim, b"untouched\n").unwrap();
    std::os::unix::fs::symlink(&victim, dir.path().join("link")).unwrap();

    let error = files::save(dir.path(), "link", b"clobbered\n", None).unwrap_err();
    assert!(refused(&error), "answered {error:?}");
    assert_eq!(fs::read(&victim).unwrap(), b"untouched\n");
}

#[test]
fn a_save_into_a_folder_that_went_away_is_refused() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("project");
    fs::create_dir(&root).unwrap();
    fs::remove_dir(&root).unwrap();

    assert_eq!(
        files::save(&root, "file.txt", b"x", None).unwrap_err(),
        FileError::Missing
    );
}

#[test]
fn a_save_never_creates_a_folder() {
    let dir = project();
    assert_eq!(
        files::save(dir.path(), "new/dir/file.txt", b"x", None).unwrap_err(),
        FileError::Missing
    );
    assert!(
        !dir.path().join("new").exists(),
        "a write brought a folder into existence"
    );
}
