//! Refs and the commit log, against a real git directory.
//!
//! Kept apart from `tests/git.rs` — that file is the working tree and the overview, this one is
//! the repository's past.

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use ubiq_host::git::{history, observe};
use ubiq_proto::git::GitRefKind;

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

fn open(dir: &Path) -> git2::Repository {
    observe::open(dir).unwrap().expect("a repository")
}

#[test]
fn the_refs_list_names_the_current_branch() {
    let dir = repository();
    let repo = open(dir.path());
    let refs = history::refs(&repo, false).unwrap();
    let current = refs.iter().find(|r| r.current).expect("a current ref");
    assert_eq!(current.name, "main");
    assert_eq!(current.kind, GitRefKind::Local);
}

#[test]
fn a_tag_is_in_the_refs_list() {
    let dir = repository();
    git(dir.path(), &["tag", "v1"]);
    let repo = open(dir.path());
    let refs = history::refs(&repo, false).unwrap();
    assert!(
        refs.iter()
            .any(|r| r.name == "v1" && r.kind == GitRefKind::Tag),
        "no v1 tag in {refs:?}"
    );
}

#[test]
fn a_log_page_returns_a_cursor_and_the_next_page_continues() {
    let dir = repository();
    for i in 0..5 {
        fs::write(dir.path().join("file.txt"), format!("v{i}\n")).unwrap();
        git(dir.path(), &["commit", "-q", "-am", &format!("commit {i}")]);
    }
    // "first" plus five more: six commits total.
    let repo = open(dir.path());

    let (page1, cursor) = history::log(&repo, "", None, 3, None, false).unwrap();
    assert_eq!(page1.len(), 3);
    let cursor = cursor.expect("three commits remain");

    let (page2, cursor2) = history::log(&repo, "", Some(&cursor), 10, None, false).unwrap();
    assert_eq!(page2.len(), 3);
    assert!(cursor2.is_none(), "the walk should have run out");

    let ids1: Vec<&str> = page1.iter().map(|c| c.id.as_str()).collect();
    assert!(
        page2.iter().all(|c| !ids1.contains(&c.id.as_str())),
        "the second page repeated a commit from the first"
    );
}

#[test]
fn a_path_filtered_log_returns_only_commits_touching_the_path() {
    let dir = repository();
    fs::write(dir.path().join("other.txt"), b"other\n").unwrap();
    git(dir.path(), &["add", "other.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "add other"]);
    fs::write(dir.path().join("file.txt"), b"changed\n").unwrap();
    git(dir.path(), &["commit", "-q", "-am", "change file"]);

    let repo = open(dir.path());
    let (page, _) = history::log(&repo, "", None, 10, Some("file.txt"), false).unwrap();
    let summaries: Vec<&str> = page.iter().map(|c| c.summary.as_str()).collect();
    assert!(
        summaries.contains(&"first") && summaries.contains(&"change file"),
        "{summaries:?}"
    );
    assert!(
        !summaries.contains(&"add other"),
        "a commit that never touched file.txt was in the page: {summaries:?}"
    );
}

#[test]
fn first_parent_skips_the_merged_side() {
    let dir = repository();
    git(dir.path(), &["checkout", "-q", "-b", "feature"]);
    fs::write(dir.path().join("feature.txt"), b"feature\n").unwrap();
    git(dir.path(), &["add", "feature.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "feature commit"]);
    git(dir.path(), &["checkout", "-q", "main"]);
    git(
        dir.path(),
        &["merge", "--no-ff", "-q", "-m", "merge feature", "feature"],
    );

    let repo = open(dir.path());
    let (page, _) = history::log(&repo, "", None, 10, None, true).unwrap();
    let summaries: Vec<&str> = page.iter().map(|c| c.summary.as_str()).collect();
    assert!(
        summaries.contains(&"merge feature"),
        "the merge commit itself should still be in the page: {summaries:?}"
    );
    assert!(
        !summaries.contains(&"feature commit"),
        "first_parent should have skipped the merged side: {summaries:?}"
    );
}

#[test]
fn an_unborn_head_returns_an_empty_page_not_an_error() {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    let repo = open(dir.path());
    let (page, cursor) = history::log(&repo, "", None, 10, None, false).unwrap();
    assert!(page.is_empty());
    assert!(cursor.is_none());
}
