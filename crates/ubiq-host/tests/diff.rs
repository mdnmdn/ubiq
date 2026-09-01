//! A file compared with what version control holds, against a real repository.
//!
//! Every test here builds a scratch repository in a temporary directory and drives the host's own
//! `git` — because what is being asserted is that the host agrees with version control, and a
//! fixture written by hand would only assert that it agrees with the fixture.

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use ubiq_host::files::diff::diff;
use ubiq_proto::files::{DiffBase, DiffRowKind, FileError};

/// Run one git command in `dir`, ignoring whatever the machine's own configuration says.
fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(dir)
        // The user running the tests has a name, a signing key and a default branch, and none of
        // them are this repository's business.
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

/// A repository with `file.txt` committed, holding six numbered lines.
fn repository() -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    fs::write(dir.path().join("file.txt"), lines(&["one", "two", "three"])).unwrap();
    git(dir.path(), &["add", "file.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "first"]);
    dir
}

fn lines(lines: &[&str]) -> String {
    lines
        .iter()
        .map(|line| format!("{line}\n"))
        .collect::<String>()
}

// ── the comparison ──────────────────────────────────────────────────

#[test]
fn a_changed_line_is_one_hunk_with_both_line_numbers() {
    let dir = repository();
    fs::write(
        dir.path().join("file.txt"),
        lines(&["one", "changed", "three"]),
    )
    .unwrap();

    let answer = diff(dir.path(), "file.txt", DiffBase::Head).unwrap();
    assert!(!answer.binary);
    assert!(!answer.truncated);
    assert_eq!(answer.base, DiffBase::Head);
    assert_eq!(answer.hunks.len(), 1);

    let hunk = &answer.hunks[0];
    assert_eq!((hunk.old_start, hunk.old_lines), (1, 3));
    assert_eq!((hunk.new_start, hunk.new_lines), (1, 3));

    let shape: Vec<(DiffRowKind, Option<u32>, Option<u32>, &str)> = hunk
        .rows
        .iter()
        .map(|row| (row.kind, row.old_line, row.new_line, row.text.as_str()))
        .collect();
    assert_eq!(
        shape,
        vec![
            (DiffRowKind::Context, Some(1), Some(1), "one"),
            (DiffRowKind::Removed, Some(2), None, "two"),
            (DiffRowKind::Added, None, Some(2), "changed"),
            (DiffRowKind::Context, Some(3), Some(3), "three"),
        ]
    );
}

#[test]
fn a_row_carries_no_marker_of_its_own() {
    let dir = repository();
    fs::write(
        dir.path().join("file.txt"),
        lines(&["one", "two", "three", "four"]),
    )
    .unwrap();

    let answer = diff(dir.path(), "file.txt", DiffBase::Head).unwrap();
    for row in answer.hunks.iter().flat_map(|hunk| &hunk.rows) {
        assert!(
            !row.text.starts_with('+') && !row.text.starts_with('-'),
            "{row:?} carries the marker its kind already is"
        );
        assert!(!row.text.ends_with('\n'), "{row:?} kept its terminator");
    }
}

#[test]
fn a_file_with_no_change_answers_with_no_hunks() {
    let dir = repository();

    for base in [DiffBase::Head, DiffBase::Index] {
        let answer = diff(dir.path(), "file.txt", base).unwrap();
        assert!(answer.hunks.is_empty(), "{base:?} answered {answer:?}");
        assert!(!answer.binary);
        assert!(!answer.truncated);
    }
}

#[test]
fn staging_moves_a_change_out_of_the_index_diff_and_leaves_it_in_head() {
    let dir = repository();
    fs::write(
        dir.path().join("file.txt"),
        lines(&["one", "changed", "three"]),
    )
    .unwrap();
    git(dir.path(), &["add", "file.txt"]);

    // Staged: the working tree and the index now agree…
    let staged = diff(dir.path(), "file.txt", DiffBase::Index).unwrap();
    assert!(staged.hunks.is_empty(), "{staged:?}");

    // …and the commit still holds the old line.
    let committed = diff(dir.path(), "file.txt", DiffBase::Head).unwrap();
    assert_eq!(committed.hunks.len(), 1);
    let texts: Vec<&str> = committed.hunks[0]
        .rows
        .iter()
        .filter(|row| row.kind == DiffRowKind::Added)
        .map(|row| row.text.as_str())
        .collect();
    assert_eq!(texts, vec!["changed"]);
}

#[test]
fn an_untracked_file_is_wholly_added_against_either_base() {
    let dir = repository();
    fs::write(dir.path().join("new.txt"), lines(&["fresh", "lines"])).unwrap();

    for base in [DiffBase::Head, DiffBase::Index] {
        let answer = diff(dir.path(), "new.txt", base).unwrap();
        assert_eq!(answer.hunks.len(), 1, "{base:?} answered {answer:?}");
        let hunk = &answer.hunks[0];
        // A side with no lines starts at the line before it, which is what `@@ -0,0 +1,2 @@` says.
        assert_eq!((hunk.old_start, hunk.old_lines), (0, 0));
        assert_eq!((hunk.new_start, hunk.new_lines), (1, 2));
        assert!(
            hunk.rows.iter().all(|row| row.kind == DiffRowKind::Added),
            "{hunk:?}"
        );
        assert_eq!(hunk.rows[0].new_line, Some(1));
        assert_eq!(hunk.rows[0].old_line, None);
    }
}

#[test]
fn an_ignored_file_is_treated_the_same_as_an_untracked_one() {
    let dir = repository();
    fs::write(dir.path().join(".gitignore"), lines(&["ignored.txt"])).unwrap();
    fs::write(dir.path().join("ignored.txt"), lines(&["invisible"])).unwrap();

    let answer = diff(dir.path(), "ignored.txt", DiffBase::Head).unwrap();
    assert_eq!(answer.hunks.len(), 1);
    assert!(
        answer.hunks[0]
            .rows
            .iter()
            .all(|row| row.kind == DiffRowKind::Added)
    );
}

#[test]
fn a_repository_with_no_commit_yet_adds_every_file() {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    fs::write(dir.path().join("first.txt"), lines(&["hello"])).unwrap();

    // An unborn branch has no tree to compare with, and refusing here would put every file in a
    // fresh repository behind an error the user cannot act on.
    let answer = diff(dir.path(), "first.txt", DiffBase::Head).unwrap();
    assert_eq!(answer.hunks.len(), 1);
    assert_eq!(answer.hunks[0].new_lines, 1);
}

#[test]
fn a_deletion_and_an_insertion_keep_their_own_line_numbers() {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    fs::write(
        dir.path().join("file.txt"),
        lines(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]),
    )
    .unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "first"]);

    // One line gone near the top, one line arrived near the bottom: far enough apart that the
    // context radius cannot join them.
    fs::write(
        dir.path().join("file.txt"),
        lines(&["a", "c", "d", "e", "f", "g", "h", "i", "extra", "j"]),
    )
    .unwrap();

    let answer = diff(dir.path(), "file.txt", DiffBase::Head).unwrap();
    assert_eq!(answer.hunks.len(), 2, "{answer:?}");

    let removed = answer.hunks[0]
        .rows
        .iter()
        .find(|row| row.kind == DiffRowKind::Removed)
        .unwrap();
    assert_eq!((removed.old_line, removed.new_line), (Some(2), None));
    assert_eq!(removed.text, "b");

    let added = answer.hunks[1]
        .rows
        .iter()
        .find(|row| row.kind == DiffRowKind::Added)
        .unwrap();
    // The insertion sits after nine lines on the new side and the old side never had it.
    assert_eq!((added.old_line, added.new_line), (None, Some(9)));
    assert_eq!(added.text, "extra");
}

// ── the ceilings and the refusals ───────────────────────────────────

#[test]
fn a_binary_file_is_reported_and_never_diffed() {
    let dir = repository();
    fs::write(dir.path().join("blob.bin"), b"before\0\x01\x02").unwrap();
    git(dir.path(), &["add", "blob.bin"]);
    git(dir.path(), &["commit", "-q", "-m", "binary"]);
    fs::write(dir.path().join("blob.bin"), b"after\0\x03\x04").unwrap();

    let answer = diff(dir.path(), "blob.bin", DiffBase::Head).unwrap();
    assert!(answer.binary);
    assert!(answer.hunks.is_empty(), "a binary file carried hunks");
}

#[test]
fn a_change_past_the_row_ceiling_says_it_is_truncated() {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    fs::write(dir.path().join("big.txt"), lines(&["tail"])).unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "first"]);

    // One insertion of more rows than an answer carries, which is the cheapest way to be past the
    // ceiling: what is asserted is the ceiling, not the comparison behind it.
    let mut grown: Vec<String> = (0..12_000).map(|n| format!("line {n}")).collect();
    grown.push("tail".to_string());
    fs::write(dir.path().join("big.txt"), lines(&refs(&grown))).unwrap();

    let answer = diff(dir.path(), "big.txt", DiffBase::Head).unwrap();
    assert!(answer.truncated, "a cut answer has to say so");
    assert!(!answer.binary);
    let rows: usize = answer.hunks.iter().map(|hunk| hunk.rows.len()).sum();
    assert!(rows > 0 && rows <= 10_000, "{rows} rows came back");
}

#[test]
fn a_side_past_the_byte_ceiling_is_truncated_rather_than_half_compared() {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    fs::write(dir.path().join("huge.txt"), lines(&["small"])).unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "first"]);

    // Just past two megabytes of ordinary text.
    let line = "x".repeat(63);
    let huge: String = std::iter::repeat_n(line.as_str(), 34_000)
        .map(|line| format!("{line}\n"))
        .collect();
    fs::write(dir.path().join("huge.txt"), &huge).unwrap();

    let answer = diff(dir.path(), "huge.txt", DiffBase::Head).unwrap();
    assert!(answer.truncated);
    // No hunks rather than two prefixes compared, which would draw the missing tail as a deletion
    // nobody made.
    assert!(answer.hunks.is_empty(), "{answer:?}");
    assert!(!answer.binary);
}

fn refs(lines: &[String]) -> Vec<&str> {
    lines.iter().map(String::as_str).collect()
}

#[test]
fn a_folder_with_no_version_control_is_refused() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("file.txt"), lines(&["alone"])).unwrap();

    let error = diff(dir.path(), "file.txt", DiffBase::Head).unwrap_err();
    // Refused, not an empty diff: an empty diff would draw as a file with no changes.
    assert!(matches!(error, FileError::Refused(_)), "answered {error:?}");
}

#[test]
fn a_path_that_is_not_a_file_is_refused_before_version_control_is_opened() {
    let dir = repository();
    fs::create_dir(dir.path().join("folder")).unwrap();

    assert_eq!(
        diff(dir.path(), "gone.txt", DiffBase::Head).unwrap_err(),
        FileError::Missing
    );
    assert_eq!(
        diff(dir.path(), "folder", DiffBase::Head).unwrap_err(),
        FileError::WrongKind
    );

    let escape = diff(dir.path(), "../elsewhere", DiffBase::Head).unwrap_err();
    assert!(
        matches!(escape, FileError::Refused(_)),
        "answered {escape:?}"
    );
}

#[test]
fn a_project_inside_a_repository_is_diffed_against_that_repository() {
    let dir = repository();
    fs::create_dir(dir.path().join("crate")).unwrap();
    fs::write(dir.path().join("crate/inner.txt"), lines(&["one"])).unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "inner"]);
    fs::write(dir.path().join("crate/inner.txt"), lines(&["two"])).unwrap();

    // The project's root is the subfolder; the repository is above it, which is the ordinary case
    // for a crate inside a workspace.
    let answer = diff(&dir.path().join("crate"), "inner.txt", DiffBase::Head).unwrap();
    assert_eq!(answer.hunks.len(), 1, "{answer:?}");
    assert_eq!(answer.hunks[0].rows.len(), 2);
}
