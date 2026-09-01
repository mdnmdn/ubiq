//! A project's repository as the host observes it, against a real git directory.
//!
//! Every test here builds a scratch repository in a temporary directory and drives the host's own
//! `git` — because what is being asserted is that the host agrees with version control, and a
//! fixture written by hand would only assert that it agrees with the fixture.

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use ubiq_host::git::observe;
use ubiq_proto::git::{GitHead, GitMark, GitPathChange};

/// Run one git command in `dir`, ignoring whatever the machine's own configuration says.
fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(dir)
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

/// A repository with `file.txt` committed on `main`.
fn repository() -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    fs::write(dir.path().join("file.txt"), b"hello\n").unwrap();
    git(dir.path(), &["add", "file.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "first"]);
    dir
}

fn entry<'a>(tree: &'a ubiq_host::git::WorkingTree, path: &str) -> &'a ubiq_proto::git::GitEntry {
    tree.entries
        .iter()
        .find(|e| e.rel_path == path)
        .unwrap_or_else(|| panic!("no entry for {path}: {:?}", tree.entries))
}

#[test]
fn a_folder_with_no_repository_answers_none() {
    let dir = TempDir::new().unwrap();
    let found = observe(dir.path(), 0, true).unwrap();
    assert!(found.overview.is_none());
    assert!(found.tree.is_none());
}

#[test]
fn a_repository_on_main_names_the_branch() {
    let dir = repository();
    let found = observe(dir.path(), 1, false).unwrap();
    let overview = found.overview.expect("a repository");
    assert_eq!(overview.head, GitHead::Branch("main".into()));
    assert!(overview.upstream.is_none());
    assert!(overview.ahead.is_none());
    assert!(overview.behind.is_none());
    assert!(overview.counts.is_none(), "an overview does not walk");
    assert!(!overview.is_bare);
    assert_eq!(overview.generation, 1);
    assert!(overview.scoped_to.is_empty());
}

#[test]
fn an_unborn_head_keeps_the_branch_name() {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    let overview = observe(dir.path(), 0, false)
        .unwrap()
        .overview
        .expect("a repository");
    assert_eq!(overview.head, GitHead::Unborn("main".into()));
    assert!(overview.counts.is_none());
}

#[test]
fn a_detached_head_draws_the_short_id() {
    let dir = repository();
    git(dir.path(), &["checkout", "-q", "--detach"]);
    let overview = observe(dir.path(), 0, false)
        .unwrap()
        .overview
        .expect("a repository");
    match overview.head {
        GitHead::Detached { short_id } => assert!(!short_id.is_empty(), "{short_id}"),
        other => panic!("expected detached, got {other:?}"),
    }
}

#[test]
fn a_modified_file_is_in_the_map() {
    let dir = repository();
    fs::write(dir.path().join("file.txt"), b"changed\n").unwrap();
    let found = observe(dir.path(), 2, true).unwrap();
    let tree = found.tree.expect("a working tree");
    let file = entry(&tree, "file.txt");
    assert_eq!(file.worktree, Some(GitPathChange::Modified));
    assert_eq!(file.index, None);
    assert_eq!(file.mark(), Some(GitMark::Modified));
    let counts = found.overview.unwrap().counts.unwrap();
    assert_eq!(counts.modified, 1);
    assert_eq!(counts.staged, 0);
    assert_eq!(counts.untracked, 0);
}

#[test]
fn an_untracked_file_is_in_the_map() {
    let dir = repository();
    fs::write(dir.path().join("new.txt"), b"new\n").unwrap();
    let tree = observe(dir.path(), 1, true)
        .unwrap()
        .tree
        .expect("a working tree");
    let file = entry(&tree, "new.txt");
    assert_eq!(file.worktree, Some(GitPathChange::Untracked));
    assert_eq!(file.mark(), Some(GitMark::Untracked));
}

#[test]
fn a_staged_file_is_in_the_map() {
    let dir = repository();
    fs::write(dir.path().join("file.txt"), b"changed\n").unwrap();
    git(dir.path(), &["add", "file.txt"]);
    let tree = observe(dir.path(), 1, true)
        .unwrap()
        .tree
        .expect("a working tree");
    let file = entry(&tree, "file.txt");
    assert_eq!(file.index, Some(GitPathChange::Modified));
    assert_eq!(file.worktree, None);
    assert_eq!(file.mark(), Some(GitMark::Staged));
}

#[test]
fn a_file_staged_and_modified_draws_as_modified() {
    let dir = repository();
    fs::write(dir.path().join("file.txt"), b"staged\n").unwrap();
    git(dir.path(), &["add", "file.txt"]);
    fs::write(dir.path().join("file.txt"), b"unstaged\n").unwrap();
    let tree = observe(dir.path(), 1, true)
        .unwrap()
        .tree
        .expect("a working tree");
    let file = entry(&tree, "file.txt");
    assert!(file.index.is_some());
    assert!(file.worktree.is_some());
    assert_eq!(file.mark(), Some(GitMark::Modified));
}

#[test]
fn a_conflicted_file_draws_as_conflict() {
    let dir = repository();
    git(dir.path(), &["checkout", "-q", "-b", "other"]);
    fs::write(dir.path().join("file.txt"), b"other\n").unwrap();
    git(dir.path(), &["commit", "-q", "-am", "other"]);
    git(dir.path(), &["checkout", "-q", "main"]);
    fs::write(dir.path().join("file.txt"), b"main\n").unwrap();
    git(dir.path(), &["commit", "-q", "-am", "main"]);
    let merge = Command::new("git")
        .current_dir(dir.path())
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .args(["merge", "--no-commit", "other"])
        .output()
        .expect("git merge");
    assert!(!merge.status.success(), "the merge should conflict");

    let found = observe(dir.path(), 1, true).unwrap();
    let file = entry(found.tree.as_ref().unwrap(), "file.txt");
    assert!(file.conflicted);
    assert_eq!(file.mark(), Some(GitMark::Conflict));
    assert_eq!(
        found.overview.unwrap().operation,
        Some(ubiq_proto::git::GitOperation::Merge)
    );
}

#[test]
fn a_project_inside_a_repository_is_scoped() {
    let dir = repository();
    fs::create_dir(dir.path().join("pkg")).unwrap();
    fs::write(dir.path().join("pkg/inner.txt"), b"inner\n").unwrap();
    git(dir.path(), &["add", "pkg/inner.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "pkg"]);
    fs::write(dir.path().join("file.txt"), b"changed\n").unwrap();
    fs::write(dir.path().join("pkg/inner.txt"), b"changed inner\n").unwrap();

    let found = observe(&dir.path().join("pkg"), 1, true).unwrap();
    let overview = found.overview.unwrap();
    assert_eq!(overview.head, GitHead::Branch("main".into()));
    assert_eq!(overview.scoped_to, "pkg");
    let tree = found.tree.unwrap();
    assert!(
        tree.entries.iter().any(|e| e.rel_path == "inner.txt"),
        "scoped entries: {:?}",
        tree.entries
    );
    assert!(
        tree.entries.iter().all(|e| e.rel_path != "file.txt"),
        "the outer file leaked into the project's map"
    );
    assert_eq!(overview.counts.unwrap().modified, 1);
}

#[test]
fn an_untracked_directory_is_one_entry() {
    let dir = repository();
    fs::create_dir(dir.path().join("fresh")).unwrap();
    fs::write(dir.path().join("fresh/a.rs"), b"a\n").unwrap();
    fs::create_dir(dir.path().join("fresh/nested")).unwrap();
    fs::write(dir.path().join("fresh/nested/b.rs"), b"b\n").unwrap();

    let tree = observe(dir.path(), 1, true)
        .unwrap()
        .tree
        .expect("a working tree");
    let file = entry(&tree, "fresh");
    assert_eq!(file.worktree, Some(GitPathChange::Untracked));
    assert_eq!(file.mark(), Some(GitMark::Untracked));
    assert!(
        tree.entries
            .iter()
            .all(|e| e.rel_path == "fresh" || !e.rel_path.starts_with("fresh")),
        "an untracked directory is one collapsed entry, not its children: {:?}",
        tree.entries
    );
}

#[test]
fn a_ds_store_is_absent_from_the_map() {
    let dir = repository();
    fs::write(dir.path().join(".DS_Store"), b"junk").unwrap();
    fs::write(dir.path().join("new.txt"), b"new\n").unwrap();
    let tree = observe(dir.path(), 1, true)
        .unwrap()
        .tree
        .expect("a working tree");
    assert!(
        tree.entries.iter().all(|e| e.rel_path != ".DS_Store"),
        "macOS junk was in the map: {:?}",
        tree.entries
    );
    assert!(tree.entries.iter().any(|e| e.rel_path == "new.txt"));
}

#[test]
fn a_changed_file_rolls_up_its_parent() {
    let dir = repository();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), b"fn main() {}\n").unwrap();
    git(dir.path(), &["add", "src/main.rs"]);
    git(dir.path(), &["commit", "-q", "-m", "src"]);
    fs::write(
        dir.path().join("src/main.rs"),
        b"fn main() { /* changed */ }\n",
    )
    .unwrap();

    let tree = observe(dir.path(), 1, true)
        .unwrap()
        .tree
        .expect("a working tree");
    assert_eq!(entry(&tree, "src/main.rs").mark(), Some(GitMark::Modified));
    let rollup = tree
        .rollups
        .iter()
        .find(|r| r.rel_path == "src")
        .expect("src should roll up");
    assert_eq!(rollup.mark, GitMark::Modified);
}
