//! A project's repository as the host observes it, against a real git directory.
//!
//! Every test here builds a scratch repository in a temporary directory and drives the host's own
//! `git` — because what is being asserted is that the host agrees with version control, and a
//! fixture written by hand would only assert that it agrees with the fixture.

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use ubiq_host::git::{history, nested, observe, write};
use ubiq_proto::git::{GitError, GitHead, GitMark, GitPathChange, GitSubmoduleState, GitWriteOp};

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

fn git_stdout(dir: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// A repository with `file.txt` committed on `main`.
fn repository() -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    identity(dir.path());
    fs::write(dir.path().join("file.txt"), b"hello\n").unwrap();
    git(dir.path(), &["add", "file.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "first"]);
    dir
}

/// libgit2 reads `user.name` / `user.email` from the repository, not from the author env.
fn identity(dir: &Path) {
    git(dir, &["config", "user.name", "Ubiq"]);
    git(dir, &["config", "user.email", "ubiq@example.invalid"]);
}

/// The managed set as a record carries it: the repositories inside the project it takes on.
fn managed(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|path| path.to_string()).collect()
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
    let found = observe(dir.path(), 0, true, &[]).unwrap();
    assert!(found.overview.is_none());
    assert!(found.tree.is_none());
}

#[test]
fn a_repository_on_main_names_the_branch() {
    let dir = repository();
    let found = observe(dir.path(), 1, false, &[]).unwrap();
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
    let overview = observe(dir.path(), 0, false, &[])
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
    let overview = observe(dir.path(), 0, false, &[])
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
    let found = observe(dir.path(), 2, true, &[]).unwrap();
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
    let tree = observe(dir.path(), 1, true, &[])
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
    let tree = observe(dir.path(), 1, true, &[])
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
    let tree = observe(dir.path(), 1, true, &[])
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

    let found = observe(dir.path(), 1, true, &[]).unwrap();
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

    let found = observe(&dir.path().join("pkg"), 1, true, &[]).unwrap();
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

    let tree = observe(dir.path(), 1, true, &[])
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
    let tree = observe(dir.path(), 1, true, &[])
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

    let tree = observe(dir.path(), 1, true, &[])
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

#[test]
fn a_remote_is_named_in_the_overview() {
    let dir = repository();
    git(
        dir.path(),
        &["remote", "add", "origin", "https://example/x"],
    );
    let overview = observe(dir.path(), 0, false, &[])
        .unwrap()
        .overview
        .expect("a repository");
    assert_eq!(overview.remotes.len(), 1);
    assert_eq!(overview.remotes[0].name, "origin");
    assert_eq!(overview.remotes[0].url, "https://example/x");
    assert!(overview.remotes[0].is_default);
}

#[test]
fn an_uninitialised_submodule_is_listed_not_walked() {
    let sub_origin = repository();
    let dir = repository();
    let sub_url = format!("file://{}", sub_origin.path().display());
    git(
        dir.path(),
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            &sub_url,
            "sub",
        ],
    );
    git(dir.path(), &["commit", "-q", "-m", "add submodule"]);
    // A fresh clone that never ran `submodule update --init` has the gitlink and `.gitmodules`
    // but nothing checked out. `deinit` reproduces exactly that, without a second clone.
    git(dir.path(), &["submodule", "deinit", "-f", "sub"]);

    let found = observe(dir.path(), 1, true, &[]).unwrap();
    let overview = found.overview.expect("a repository");
    assert_eq!(overview.submodules.len(), 1);
    assert_eq!(overview.submodules[0].rel_path, "sub");
    assert_eq!(
        overview.submodules[0].state,
        GitSubmoduleState::Uninitialised
    );

    let tree = found.tree.expect("a working tree");
    assert!(
        tree.entries.iter().all(|e| !e.rel_path.starts_with("sub")),
        "an uninitialised submodule leaked into the working tree: {:?}",
        tree.entries
    );
}

#[test]
fn an_ignored_directory_is_one_entry() {
    let dir = repository();
    fs::write(dir.path().join(".gitignore"), b"target/\n").unwrap();
    git(dir.path(), &["add", ".gitignore"]);
    git(dir.path(), &["commit", "-q", "-m", "ignore target"]);
    fs::create_dir(dir.path().join("target")).unwrap();
    fs::write(dir.path().join("target/a"), b"a\n").unwrap();
    fs::write(dir.path().join("target/b"), b"b\n").unwrap();

    let tree = observe(dir.path(), 1, true, &[])
        .unwrap()
        .tree
        .expect("a working tree");
    let matches: Vec<_> = tree
        .entries
        .iter()
        .filter(|e| e.rel_path.starts_with("target"))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "an ignored directory should collapse to one entry: {:?}",
        tree.entries
    );
    assert_eq!(matches[0].rel_path, "target");
    assert!(matches[0].ignored);
}

#[test]
fn an_ignored_directory_does_not_outrank_a_modified_sibling() {
    let dir = repository();
    fs::create_dir(dir.path().join("area")).unwrap();
    fs::write(dir.path().join("area/file.txt"), b"first\n").unwrap();
    git(dir.path(), &["add", "area/file.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "area"]);
    fs::write(dir.path().join(".gitignore"), b"area/target/\n").unwrap();
    git(dir.path(), &["add", ".gitignore"]);
    git(dir.path(), &["commit", "-q", "-m", "ignore area/target"]);

    fs::create_dir(dir.path().join("area/target")).unwrap();
    fs::write(dir.path().join("area/target/a"), b"a\n").unwrap();
    fs::write(dir.path().join("area/file.txt"), b"changed\n").unwrap();

    let tree = observe(dir.path(), 1, true, &[])
        .unwrap()
        .tree
        .expect("a working tree");
    let rollup = tree
        .rollups
        .iter()
        .find(|r| r.rel_path == "area")
        .expect("area should roll up");
    assert_eq!(
        rollup.mark,
        GitMark::Modified,
        "the ignored sibling outranked the modified file"
    );
}

// ── Repositories inside the project ─────────────────────────────
//
// A project may hold repositories below it: submodules the outer one pins, and independent trees
// it knows only as one untracked folder. One the project *manages* is walked and merged into the
// one project-relative map (`D99`), so these assert on `rel_path`s that carry the nested root as a
// prefix. One it does not manage is named and nothing more, which is what a fresh discovery is.

/// An independent clone inside the project reports its own file statuses, not one untracked folder.
#[test]
fn a_nested_repository_is_walked_and_merged() {
    let dir = repository();
    let inner = dir.path().join("inner");
    fs::create_dir(&inner).unwrap();
    git(&inner, &["init", "-q", "-b", "main"]);
    fs::write(inner.join("kept.txt"), b"kept\n").unwrap();
    git(&inner, &["add", "kept.txt"]);
    git(&inner, &["commit", "-q", "-m", "inner first"]);
    fs::write(inner.join("kept.txt"), b"changed\n").unwrap();
    fs::write(inner.join("fresh.txt"), b"fresh\n").unwrap();

    let found = observe(dir.path(), 1, true, &managed(&["inner"])).unwrap();
    let tree = found.tree.expect("a working tree");
    assert_eq!(
        entry(&tree, "inner/kept.txt").mark(),
        Some(GitMark::Modified)
    );
    assert_eq!(
        entry(&tree, "inner/fresh.txt").mark(),
        Some(GitMark::Untracked)
    );
    assert!(
        tree.entries.iter().all(|e| e.rel_path != "inner"),
        "the nested clone was reported as one untracked folder: {:?}",
        tree.entries
    );
    let rollup = tree
        .rollups
        .iter()
        .find(|r| r.rel_path == "inner")
        .expect("inner should roll up from the nested repository's own changes");
    assert_eq!(rollup.mark, GitMark::Modified);

    let nested = tree
        .repos
        .iter()
        .find(|r| r.rel_path == "inner")
        .expect("the nested repository should be named");
    assert_eq!(nested.head, GitHead::Branch("main".into()));
    assert!(!nested.submodule, "an independent clone is not a submodule");
    assert!(nested.managed, "the project took this one on");
    let counts = nested.counts.expect("a readable repository has counts");
    assert_eq!(counts.modified, 1);
    assert_eq!(counts.untracked, 1);

    // The project's own counts stay the outer repository's own.
    let outer = found.overview.unwrap().counts.unwrap();
    assert_eq!(outer.modified, 0);
    assert_eq!(outer.untracked, 0);
}

/// A repository with a dirty file and a nested clone beside it, for the managed-set tests.
fn with_a_nested_clone() -> TempDir {
    let dir = repository();
    let inner = dir.path().join("inner");
    fs::create_dir(&inner).unwrap();
    git(&inner, &["init", "-q", "-b", "main"]);
    fs::write(inner.join("kept.txt"), b"kept\n").unwrap();
    git(&inner, &["add", "kept.txt"]);
    git(&inner, &["commit", "-q", "-m", "inner first"]);
    fs::write(inner.join("kept.txt"), b"changed\n").unwrap();
    dir
}

/// A repository the project does not manage is named and read no further.
#[test]
fn an_unmanaged_nested_repository_is_named_and_not_walked() {
    let dir = with_a_nested_clone();

    let tree = observe(dir.path(), 1, true, &[])
        .unwrap()
        .tree
        .expect("a working tree");
    let nested = tree
        .repos
        .iter()
        .find(|r| r.rel_path == "inner")
        .expect("every repository found is named, managed or not");
    assert!(!nested.managed, "a discovery defaults to ignored");
    assert!(
        nested.counts.is_none(),
        "nothing is read from a repository that is not opened"
    );
    assert!(
        tree.entries
            .iter()
            .all(|e| !e.rel_path.starts_with("inner")),
        "an unmanaged repository contributes no entries: {:?}",
        tree.entries
    );
    assert!(
        tree.rollups
            .iter()
            .all(|r| !r.rel_path.starts_with("inner")),
        "and no rollups: {:?}",
        tree.rollups
    );
}

/// The same repository, once the project manages it, is walked and merged.
#[test]
fn a_managed_nested_repository_joins_the_map() {
    let dir = with_a_nested_clone();

    let tree = observe(dir.path(), 1, true, &managed(&["inner"]))
        .unwrap()
        .tree
        .expect("a working tree");
    let nested = tree
        .repos
        .iter()
        .find(|r| r.rel_path == "inner")
        .expect("the repository is still named");
    assert!(nested.managed);
    assert_eq!(nested.counts.expect("counts").modified, 1);
    assert_eq!(
        entry(&tree, "inner/kept.txt").mark(),
        Some(GitMark::Modified)
    );
}

/// An empty managed set says nothing about the repository the project *is*.
#[test]
fn an_empty_managed_set_leaves_the_project_s_own_repository_alone() {
    let dir = with_a_nested_clone();
    fs::write(dir.path().join("file.txt"), b"changed\n").unwrap();

    let found = observe(dir.path(), 1, true, &[]).unwrap();
    let overview = found.overview.expect("the project's own repository");
    assert_eq!(overview.head, GitHead::Branch("main".into()));
    assert_eq!(overview.counts.expect("counts").modified, 1);
    let tree = found.tree.expect("a working tree");
    assert_eq!(entry(&tree, "file.txt").mark(), Some(GitMark::Modified));
}

/// A folder that is no repository itself but holds one still answers a working tree.
#[test]
fn a_project_with_only_nested_repositories_answers_a_tree() {
    let dir = TempDir::new().unwrap();
    let inner = dir.path().join("clone");
    fs::create_dir(&inner).unwrap();
    git(&inner, &["init", "-q", "-b", "main"]);
    fs::write(inner.join("new.txt"), b"new\n").unwrap();

    let found = observe(dir.path(), 1, true, &managed(&["clone"])).unwrap();
    assert!(
        found.overview.is_none(),
        "there is no repository of its own"
    );
    let tree = found.tree.expect("a working tree from the nested clone");
    assert_eq!(
        entry(&tree, "clone/new.txt").mark(),
        Some(GitMark::Untracked)
    );
    assert_eq!(tree.repos.len(), 1);
    assert_eq!(tree.repos[0].rel_path, "clone");
    assert_eq!(tree.repos[0].head, GitHead::Unborn("main".into()));
}

/// An initialised submodule is both a pin on the overview and a repository in the map.
#[test]
fn an_initialised_submodule_is_named_as_nested() {
    let sub_origin = repository();
    let dir = repository();
    let sub_url = format!("file://{}", sub_origin.path().display());
    git(
        dir.path(),
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            &sub_url,
            "sub",
        ],
    );
    git(dir.path(), &["commit", "-q", "-m", "add submodule"]);
    fs::write(dir.path().join("sub/loose.txt"), b"loose\n").unwrap();

    let found = observe(dir.path(), 1, true, &managed(&["sub"])).unwrap();
    let tree = found.tree.expect("a working tree");
    let nested = tree
        .repos
        .iter()
        .find(|r| r.rel_path == "sub")
        .expect("the submodule is a repository inside the project");
    assert!(nested.submodule, "the outer repository pins this one");
    assert_eq!(nested.counts.expect("counts").untracked, 1);
    assert_eq!(
        entry(&tree, "sub/loose.txt").mark(),
        Some(GitMark::Untracked)
    );
    assert_eq!(
        found.overview.unwrap().submodules[0].rel_path,
        "sub",
        "the pin is still listed on the overview"
    );
}

/// A folder holding a `.git` but nothing readable is reported without counts, not as an error.
#[test]
fn a_broken_nested_repository_keeps_the_project_answering() {
    let dir = repository();
    let broken = dir.path().join("broken");
    fs::create_dir(&broken).unwrap();
    fs::write(broken.join(".git"), b"gitdir: /nowhere/at/all\n").unwrap();
    fs::write(dir.path().join("file.txt"), b"changed\n").unwrap();

    let tree = observe(dir.path(), 1, true, &managed(&["broken"]))
        .unwrap()
        .tree
        .expect("a working tree");
    assert_eq!(entry(&tree, "file.txt").mark(), Some(GitMark::Modified));
    let nested = tree
        .repos
        .iter()
        .find(|r| r.rel_path == "broken")
        .expect("the broken clone is still named");
    assert!(
        nested.counts.is_none(),
        "a repository that will not open has no counts"
    );
}

/// The nested-root ceiling holds, and says so.
#[test]
fn the_nested_root_ceiling_holds() {
    let dir = TempDir::new().unwrap();
    for index in 0..(ubiq_proto::git::MAX_NESTED_REPOS + 3) {
        let inner = dir.path().join(format!("r{index:02}"));
        fs::create_dir(&inner).unwrap();
        git(&inner, &["init", "-q", "-b", "main"]);
    }
    let found = nested::discover(dir.path());
    assert_eq!(found.roots.len(), ubiq_proto::git::MAX_NESTED_REPOS);
    assert!(found.truncated);
}

/// The depth ceiling holds: a repository below it is not looked for.
#[test]
fn the_depth_ceiling_holds() {
    let dir = TempDir::new().unwrap();
    let mut deep = dir.path().to_path_buf();
    for level in 0..(nested::MAX_NESTED_DEPTH + 1) {
        deep = deep.join(format!("d{level}"));
    }
    fs::create_dir_all(&deep).unwrap();
    git(&deep, &["init", "-q", "-b", "main"]);

    let found = nested::discover(dir.path());
    assert!(
        found.roots.is_empty(),
        "a repository past the depth ceiling was walked: {:?}",
        found.roots
    );
    assert!(found.truncated, "a ceiling reached is reported");
}

/// A repository inside a nested repository is that repository's business, not the project's.
#[test]
fn discovery_does_not_descend_into_a_repository() {
    let dir = TempDir::new().unwrap();
    let outer = dir.path().join("outer");
    fs::create_dir_all(outer.join("deeper")).unwrap();
    git(&outer, &["init", "-q", "-b", "main"]);
    git(&outer.join("deeper"), &["init", "-q", "-b", "main"]);

    let found = nested::discover(dir.path());
    assert_eq!(found.roots, vec!["outer".to_string()]);
    assert!(!found.truncated);
}

fn tree_of(dir: &TempDir) -> ubiq_host::git::WorkingTree {
    observe(dir.path(), 1, true, &[])
        .unwrap()
        .tree
        .expect("a working tree")
}

#[test]
fn staging_an_untracked_file_tracks_it() {
    let dir = repository();
    fs::write(dir.path().join("new.txt"), b"new\n").unwrap();
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Stage {
            rel_path: "new.txt".into(),
        },
    )
    .unwrap();
    let tree = tree_of(&dir);
    let file = entry(&tree, "new.txt");
    assert_eq!(file.index, Some(GitPathChange::Added));
    assert_eq!(file.worktree, None);
    assert_eq!(file.mark(), Some(GitMark::Staged));
}

#[test]
fn unstaging_a_new_file_makes_it_untracked() {
    let dir = repository();
    fs::write(dir.path().join("new.txt"), b"new\n").unwrap();
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Stage {
            rel_path: "new.txt".into(),
        },
    )
    .unwrap();
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Unstage {
            rel_path: "new.txt".into(),
        },
    )
    .unwrap();
    let tree = tree_of(&dir);
    let file = entry(&tree, "new.txt");
    assert_eq!(file.worktree, Some(GitPathChange::Untracked));
    assert_eq!(file.index, None);
}

#[test]
fn staging_a_modified_file_puts_it_in_the_index() {
    let dir = repository();
    fs::write(dir.path().join("file.txt"), b"changed\n").unwrap();
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Stage {
            rel_path: "file.txt".into(),
        },
    )
    .unwrap();
    let tree = tree_of(&dir);
    let file = entry(&tree, "file.txt");
    assert_eq!(file.index, Some(GitPathChange::Modified));
    assert_eq!(file.worktree, None);
}

#[test]
fn stage_all_picks_up_a_modified_and_an_untracked_file() {
    let dir = repository();
    fs::write(dir.path().join("file.txt"), b"changed\n").unwrap();
    fs::write(dir.path().join("new.txt"), b"new\n").unwrap();
    write::apply_at(dir.path(), &[], &GitWriteOp::StageAll).unwrap();
    let tree = tree_of(&dir);
    let modified = entry(&tree, "file.txt");
    assert_eq!(modified.index, Some(GitPathChange::Modified));
    assert_eq!(modified.worktree, None);
    let added = entry(&tree, "new.txt");
    assert_eq!(added.index, Some(GitPathChange::Added));
    assert_eq!(added.worktree, None);
}

#[test]
fn unstage_all_restores_the_index_to_head() {
    let dir = repository();
    fs::write(dir.path().join("file.txt"), b"changed\n").unwrap();
    fs::write(dir.path().join("new.txt"), b"new\n").unwrap();
    write::apply_at(dir.path(), &[], &GitWriteOp::StageAll).unwrap();
    write::apply_at(dir.path(), &[], &GitWriteOp::UnstageAll).unwrap();
    let tree = tree_of(&dir);
    let modified = entry(&tree, "file.txt");
    assert_eq!(modified.index, None);
    assert_eq!(modified.worktree, Some(GitPathChange::Modified));
    let added = entry(&tree, "new.txt");
    assert_eq!(added.worktree, Some(GitPathChange::Untracked));
    assert_eq!(added.index, None);
}

/// The scoped branch of `StageAll`/`UnstageAll` builds a literal directory-prefix pathspec
/// (`observe::scope`, no wildcard) rather than the whole-repo `"*"`. `reset_default` diffs the
/// commit tree against the index through the same libgit2 pathspec-prefix machinery that made the
/// whole-repo `"."` a silent no-op — this asserts the scoped case actually clears the index rather
/// than trusting that a different pathspec string is fine by inspection.
#[test]
fn unstage_all_restores_the_index_to_head_when_scoped_to_a_nested_project() {
    let dir = repository();
    fs::create_dir(dir.path().join("pkg")).unwrap();
    fs::write(dir.path().join("pkg/inner.txt"), b"inner\n").unwrap();
    git(dir.path(), &["add", "pkg/inner.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "pkg"]);

    let scoped_root = dir.path().join("pkg");
    fs::write(scoped_root.join("inner.txt"), b"changed inner\n").unwrap();
    fs::write(scoped_root.join("new.txt"), b"new\n").unwrap();
    write::apply_at(&scoped_root, &[], &GitWriteOp::StageAll).unwrap();

    // Assert staging actually happened before asserting unstaging undoes it, so a failure here
    // points at the right half.
    let repo = observe::open(&scoped_root).unwrap().expect("a repository");
    let staged_after_stage: Vec<String> = repo
        .index()
        .unwrap()
        .iter()
        .filter_map(|e| std::str::from_utf8(&e.path).ok().map(str::to_string))
        .collect();
    assert!(
        staged_after_stage.contains(&"pkg/new.txt".to_string()),
        "scoped stage_all should have staged the new file: {staged_after_stage:?}"
    );

    write::apply_at(&scoped_root, &[], &GitWriteOp::UnstageAll).unwrap();

    // Read the index directly through git2 rather than through the host's own `observe`, so the
    // assertion does not share a bug with the code it is checking. `pkg/inner.txt` is a tracked
    // file, so unstaging it resets its index blob back to HEAD's rather than removing the entry —
    // the assertion is on content, not presence. `pkg/new.txt` was never in HEAD, so unstaging it
    // does remove the entry.
    let repo = observe::open(&scoped_root).unwrap().expect("a repository");
    let index = repo.index().unwrap();
    let entries: Vec<(String, git2::Oid)> = index
        .iter()
        .filter_map(|e| {
            std::str::from_utf8(&e.path)
                .ok()
                .map(|path| (path.to_string(), e.id))
        })
        .collect();
    let inner = entries
        .iter()
        .find(|(path, _)| path == "pkg/inner.txt")
        .expect("pkg/inner.txt should still be tracked in the index");
    let blob = repo.find_blob(inner.1).unwrap();
    assert_eq!(
        blob.content(),
        b"inner\n",
        "pkg/inner.txt's staged content should have reverted to HEAD's, not stayed at the \
         working-tree edit"
    );
    assert!(
        !entries.iter().any(|(path, _)| path == "pkg/new.txt"),
        "pkg/new.txt is still staged after a scoped unstage-all: {entries:?}"
    );

    let found = observe(&scoped_root, 1, true, &[]).unwrap();
    let tree = found.tree.expect("a working tree");
    let modified = entry(&tree, "inner.txt");
    assert_eq!(modified.index, None, "inner.txt should no longer be staged");
    assert_eq!(modified.worktree, Some(GitPathChange::Modified));
    let added = entry(&tree, "new.txt");
    assert_eq!(added.index, None, "new.txt should no longer be staged");
    assert_eq!(added.worktree, Some(GitPathChange::Untracked));
}

#[test]
fn changed_lists_the_modified_path_between_two_commits() {
    let dir = repository();
    let from = git_stdout(dir.path(), &["rev-parse", "HEAD"]);
    fs::write(dir.path().join("file.txt"), b"changed\n").unwrap();
    git(dir.path(), &["commit", "-q", "-am", "second"]);
    let to = git_stdout(dir.path(), &["rev-parse", "HEAD"]);
    let repo = observe::open(dir.path()).unwrap().expect("a repository");
    let files = history::changed(&repo, dir.path(), Some(&from), &to).unwrap();
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(files[0].rel_path, "file.txt");
    assert_eq!(files[0].change, GitPathChange::Modified);
}

#[test]
fn changed_reports_a_rename() {
    let dir = repository();
    let from = git_stdout(dir.path(), &["rev-parse", "HEAD"]);
    git(dir.path(), &["mv", "file.txt", "renamed.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "rename"]);
    let to = git_stdout(dir.path(), &["rev-parse", "HEAD"]);
    let repo = observe::open(dir.path()).unwrap().expect("a repository");
    let files = history::changed(&repo, dir.path(), Some(&from), &to).unwrap();
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(files[0].rel_path, "renamed.txt");
    assert_eq!(
        files[0].change,
        GitPathChange::Renamed {
            from: "file.txt".into()
        }
    );
}

#[test]
fn a_commit_clears_the_index() {
    let dir = repository();
    fs::write(dir.path().join("file.txt"), b"changed\n").unwrap();
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Stage {
            rel_path: "file.txt".into(),
        },
    )
    .unwrap();
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Commit {
            message: "second".into(),
            amend: false,
        },
    )
    .unwrap();
    let found = observe(dir.path(), 1, true, &[]).unwrap();
    assert!(
        found
            .tree
            .map(|tree| tree.entries.is_empty())
            .unwrap_or(true),
        "the working tree is clean after a commit"
    );
    let overview = found.overview.expect("a repository");
    match overview.head {
        GitHead::Branch(name) => assert_eq!(name, "main"),
        other => panic!("expected main, got {other:?}"),
    }
}

#[test]
fn an_empty_message_is_refused() {
    let dir = repository();
    let error = write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Commit {
            message: "   ".into(),
            amend: false,
        },
    )
    .unwrap_err();
    assert!(
        matches!(error, ubiq_proto::git::GitError::Failed(reason) if reason.contains("message"))
    );
}

#[test]
fn create_branch_checks_out_a_new_branch_at_head() {
    let dir = repository();
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::CreateBranch {
            name: "feature/x".into(),
        },
    )
    .unwrap();
    assert_eq!(
        git_stdout(dir.path(), &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feature/x"
    );
}

#[test]
fn create_branch_refuses_an_existing_or_invalid_name() {
    let dir = repository();
    for name in ["main", "bad name", "  "] {
        let error = write::apply_at(
            dir.path(),
            &[],
            &GitWriteOp::CreateBranch { name: name.into() },
        )
        .unwrap_err();
        assert!(matches!(error, GitError::Failed(_)), "{name:?}: {error:?}");
    }
}

#[test]
fn stash_takes_modified_and_untracked_files() {
    let dir = repository();
    fs::write(dir.path().join("file.txt"), b"changed\n").unwrap();
    fs::write(dir.path().join("new.txt"), b"new\n").unwrap();
    write::apply_at(dir.path(), &[], &GitWriteOp::Stash).unwrap();
    assert_eq!(fs::read(dir.path().join("file.txt")).unwrap(), b"hello\n");
    assert!(!dir.path().join("new.txt").exists());
    assert_eq!(
        git_stdout(dir.path(), &["stash", "list"]).lines().count(),
        1
    );
}

#[test]
fn stash_of_a_clean_tree_is_refused() {
    let dir = repository();
    let error = write::apply_at(dir.path(), &[], &GitWriteOp::Stash).unwrap_err();
    assert!(matches!(error, GitError::Failed(reason) if reason.contains("nothing")));
}

#[test]
fn undo_commit_returns_the_changes_to_the_index() {
    let dir = repository();
    fs::write(dir.path().join("file.txt"), b"second\n").unwrap();
    git(dir.path(), &["commit", "-q", "-a", "-m", "second"]);
    write::apply_at(dir.path(), &[], &GitWriteOp::UndoCommit).unwrap();
    assert_eq!(git_stdout(dir.path(), &["log", "--format=%s"]), "first");
    assert_eq!(
        git_stdout(dir.path(), &["diff", "--cached", "--name-only"]),
        "file.txt"
    );
    assert_eq!(fs::read(dir.path().join("file.txt")).unwrap(), b"second\n");
}

#[test]
fn undo_commit_refuses_a_root_commit() {
    let dir = repository();
    let error = write::apply_at(dir.path(), &[], &GitWriteOp::UndoCommit).unwrap_err();
    assert!(matches!(error, GitError::Failed(_)));
}

#[test]
fn checkout_switches_the_branch() {
    let dir = repository();
    git(dir.path(), &["branch", "feature/x"]);
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Checkout {
            rev: "feature/x".into(),
            force: false,
        },
    )
    .unwrap();
    assert_eq!(
        git_stdout(dir.path(), &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feature/x"
    );
}

#[test]
fn checkout_without_force_refuses_a_conflicting_dirty_tree() {
    let dir = repository();
    git(dir.path(), &["checkout", "-q", "-b", "feature/x"]);
    fs::write(dir.path().join("file.txt"), b"on feature\n").unwrap();
    git(dir.path(), &["commit", "-q", "-a", "-m", "on feature"]);
    git(dir.path(), &["checkout", "-q", "main"]);
    fs::write(dir.path().join("file.txt"), b"dirty\n").unwrap();
    let error = write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Checkout {
            rev: "feature/x".into(),
            force: false,
        },
    )
    .unwrap_err();
    // The UI's `checkout_conflict` (crates/ubiq/src/state/git.rs) matches this failure's reason
    // on the word "conflict", case-insensitively, to decide whether to offer a forced retry.
    // libgit2 words it "N conflict(s) prevent(s) checkout" — pin that word here so a libgit2
    // wording change fails this test loudly instead of silently dropping the UI's force-checkout
    // confirm.
    let GitError::Failed(reason) = &error else {
        panic!("expected GitError::Failed, got {error:?}");
    };
    assert!(
        reason.to_ascii_lowercase().contains("conflict"),
        "expected the refusal to mention a conflict, got {reason:?}"
    );
    assert_eq!(fs::read(dir.path().join("file.txt")).unwrap(), b"dirty\n");
}

#[test]
fn checkout_with_force_discards_local_changes() {
    let dir = repository();
    git(dir.path(), &["checkout", "-q", "-b", "feature/x"]);
    fs::write(dir.path().join("file.txt"), b"on feature\n").unwrap();
    git(dir.path(), &["commit", "-q", "-a", "-m", "on feature"]);
    git(dir.path(), &["checkout", "-q", "main"]);
    fs::write(dir.path().join("file.txt"), b"dirty\n").unwrap();
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Checkout {
            rev: "feature/x".into(),
            force: true,
        },
    )
    .unwrap();
    assert_eq!(
        git_stdout(dir.path(), &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feature/x"
    );
    assert_eq!(
        fs::read(dir.path().join("file.txt")).unwrap(),
        b"on feature\n"
    );
}

#[test]
fn checkout_a_commit_detaches_head() {
    let dir = repository();
    let sha = git_stdout(dir.path(), &["rev-parse", "HEAD"]);
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Checkout {
            rev: sha,
            force: false,
        },
    )
    .unwrap();
    let symbolic = Command::new("git")
        .current_dir(dir.path())
        .args(["symbolic-ref", "-q", "HEAD"])
        .output()
        .expect("git");
    assert!(!symbolic.status.success(), "HEAD should be detached");
}

#[test]
fn merge_fast_forwards_when_possible() {
    let dir = repository();
    git(dir.path(), &["checkout", "-q", "-b", "feature/x"]);
    fs::write(dir.path().join("file.txt"), b"on feature\n").unwrap();
    git(dir.path(), &["commit", "-q", "-a", "-m", "on feature"]);
    git(dir.path(), &["checkout", "-q", "main"]);
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Merge {
            rev: "feature/x".into(),
        },
    )
    .unwrap();
    assert_eq!(
        fs::read(dir.path().join("file.txt")).unwrap(),
        b"on feature\n"
    );
}

#[test]
fn merge_with_conflicts_fails_and_leaves_them_for_resolution() {
    let dir = repository();
    git(dir.path(), &["checkout", "-q", "-b", "feature/x"]);
    fs::write(dir.path().join("file.txt"), b"from feature\n").unwrap();
    git(dir.path(), &["commit", "-q", "-a", "-m", "on feature"]);
    git(dir.path(), &["checkout", "-q", "main"]);
    fs::write(dir.path().join("file.txt"), b"from main\n").unwrap();
    git(dir.path(), &["commit", "-q", "-a", "-m", "on main"]);
    let error = write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Merge {
            rev: "feature/x".into(),
        },
    )
    .unwrap_err();
    assert!(matches!(error, GitError::Failed(_)));
    let repo = observe::open(dir.path()).unwrap().expect("a repository");
    assert_eq!(repo.state(), git2::RepositoryState::Merge);
}

#[test]
fn merge_already_up_to_date_is_refused() {
    let dir = repository();
    git(dir.path(), &["branch", "feature/x"]);
    let error = write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Merge {
            rev: "feature/x".into(),
        },
    )
    .unwrap_err();
    assert!(matches!(error, GitError::Failed(reason) if reason.contains("up to date")));
}

#[test]
fn delete_ref_removes_a_merged_branch() {
    let dir = repository();
    git(dir.path(), &["branch", "feature/x"]);
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::DeleteRef {
            name: "feature/x".into(),
            force: false,
        },
    )
    .unwrap();
    assert!(!git_stdout(dir.path(), &["branch", "--list", "feature/x"]).contains("feature/x"));
}

#[test]
fn delete_ref_refuses_an_unmerged_branch_without_force() {
    let dir = repository();
    git(dir.path(), &["checkout", "-q", "-b", "feature/x"]);
    fs::write(dir.path().join("file.txt"), b"on feature\n").unwrap();
    git(dir.path(), &["commit", "-q", "-a", "-m", "on feature"]);
    git(dir.path(), &["checkout", "-q", "main"]);
    let error = write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::DeleteRef {
            name: "feature/x".into(),
            force: false,
        },
    )
    .unwrap_err();
    assert!(matches!(error, GitError::Failed(reason) if reason.contains("merged")));
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::DeleteRef {
            name: "feature/x".into(),
            force: true,
        },
    )
    .unwrap();
    assert!(!git_stdout(dir.path(), &["branch", "--list", "feature/x"]).contains("feature/x"));
}

#[test]
fn delete_ref_refuses_the_current_branch() {
    let dir = repository();
    let error = write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::DeleteRef {
            name: "main".into(),
            force: true,
        },
    )
    .unwrap_err();
    assert!(matches!(error, GitError::Failed(reason) if reason.contains("current branch")));
}

#[test]
fn delete_ref_removes_a_tag() {
    let dir = repository();
    git(dir.path(), &["tag", "v1.0.0"]);
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::DeleteRef {
            name: "v1.0.0".into(),
            force: false,
        },
    )
    .unwrap();
    assert_eq!(git_stdout(dir.path(), &["tag", "-l"]), "");
}

#[test]
fn delete_ref_names_an_unknown_ref() {
    let dir = repository();
    let error = write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::DeleteRef {
            name: "nope".into(),
            force: false,
        },
    )
    .unwrap_err();
    assert!(matches!(error, GitError::Failed(reason) if reason.contains("nope")));
}

#[test]
fn discard_restores_a_tracked_file_and_its_index() {
    let dir = repository();
    fs::write(dir.path().join("file.txt"), b"changed\n").unwrap();
    git(dir.path(), &["add", "file.txt"]);
    fs::write(dir.path().join("file.txt"), b"changed again\n").unwrap();
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Discard {
            rel_path: "file.txt".into(),
        },
    )
    .unwrap();
    assert_eq!(fs::read(dir.path().join("file.txt")).unwrap(), b"hello\n");
    assert_eq!(git_stdout(dir.path(), &["diff", "--name-only"]), "");
    assert_eq!(
        git_stdout(dir.path(), &["diff", "--cached", "--name-only"]),
        ""
    );
}

#[test]
fn discard_removes_an_untracked_file() {
    let dir = repository();
    fs::write(dir.path().join("new.txt"), b"new\n").unwrap();
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Discard {
            rel_path: "new.txt".into(),
        },
    )
    .unwrap();
    assert!(!dir.path().join("new.txt").exists());
}

#[test]
fn discard_removes_an_untracked_directory() {
    // `observe.rs` sets `recurse_untracked_dirs(false)`, so an untracked directory reaches the
    // changes list as one row with its trailing slash stripped — `discard` must be able to clear
    // the whole tree, not just a lone file.
    let dir = repository();
    fs::create_dir_all(dir.path().join("new_dir/nested")).unwrap();
    fs::write(dir.path().join("new_dir/a.txt"), b"a\n").unwrap();
    fs::write(dir.path().join("new_dir/nested/b.txt"), b"b\n").unwrap();
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Discard {
            rel_path: "new_dir".into(),
        },
    )
    .unwrap();
    assert!(!dir.path().join("new_dir").exists());
}

#[test]
fn restore_file_writes_the_revs_content_as_an_uncommitted_change() {
    let dir = repository();
    let first = git_stdout(dir.path(), &["rev-parse", "HEAD"]);
    fs::write(dir.path().join("file.txt"), b"second\n").unwrap();
    git(dir.path(), &["commit", "-q", "-a", "-m", "second"]);
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::RestoreFile {
            rel_path: "file.txt".into(),
            rev: first,
        },
    )
    .unwrap();
    assert_eq!(fs::read(dir.path().join("file.txt")).unwrap(), b"hello\n");
    assert_eq!(git_stdout(dir.path(), &["diff", "--name-only"]), "file.txt");
    assert_eq!(
        git_stdout(dir.path(), &["diff", "--cached", "--name-only"]),
        ""
    );
}

#[cfg(unix)]
#[test]
fn restore_file_keeps_a_symlink_a_symlink() {
    use std::os::unix::fs::symlink;

    let dir = repository();
    symlink("file.txt", dir.path().join("link.txt")).unwrap();
    git(dir.path(), &["add", "link.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "add the symlink"]);
    let first = git_stdout(dir.path(), &["rev-parse", "HEAD"]);

    fs::remove_file(dir.path().join("link.txt")).unwrap();
    fs::write(dir.path().join("link.txt"), b"not a link anymore\n").unwrap();
    git(dir.path(), &["add", "link.txt"]);
    git(
        dir.path(),
        &["commit", "-q", "-m", "replace it with a file"],
    );

    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::RestoreFile {
            rel_path: "link.txt".into(),
            rev: first,
        },
    )
    .unwrap();

    let meta = fs::symlink_metadata(dir.path().join("link.txt")).unwrap();
    assert!(
        meta.file_type().is_symlink(),
        "link.txt should be a symlink again"
    );
    assert_eq!(
        fs::read_link(dir.path().join("link.txt")).unwrap(),
        Path::new("file.txt")
    );
    assert_eq!(
        git_stdout(dir.path(), &["diff", "--cached", "--name-only"]),
        ""
    );
}

#[cfg(unix)]
#[test]
fn restore_file_keeps_the_executable_bit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = repository();
    fs::write(dir.path().join("run.sh"), b"#!/bin/sh\necho hi\n").unwrap();
    fs::set_permissions(dir.path().join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    git(dir.path(), &["add", "run.sh"]);
    git(dir.path(), &["commit", "-q", "-m", "add the executable"]);
    let first = git_stdout(dir.path(), &["rev-parse", "HEAD"]);

    // Delete it, then bring it back with a later commit that never re-marked it executable —
    // matching how a worktree file can go missing before `restore_file` is asked to bring it
    // back from an earlier rev.
    fs::remove_file(dir.path().join("run.sh")).unwrap();

    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::RestoreFile {
            rel_path: "run.sh".into(),
            rev: first,
        },
    )
    .unwrap();

    let mode = fs::metadata(dir.path().join("run.sh"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111, "run.sh should keep its executable bit");
    assert_eq!(
        git_stdout(dir.path(), &["diff", "--cached", "--name-only"]),
        ""
    );
}

#[test]
fn cherry_pick_applies_a_commits_changes() {
    let dir = repository();
    git(dir.path(), &["checkout", "-q", "-b", "feature/x"]);
    fs::write(dir.path().join("file.txt"), b"from feature\n").unwrap();
    git(dir.path(), &["commit", "-q", "-a", "-m", "on feature"]);
    let sha = git_stdout(dir.path(), &["rev-parse", "HEAD"]);
    git(dir.path(), &["checkout", "-q", "main"]);
    write::apply_at(dir.path(), &[], &GitWriteOp::CherryPick { sha }).unwrap();
    assert_eq!(
        fs::read(dir.path().join("file.txt")).unwrap(),
        b"from feature\n"
    );
    assert_eq!(
        git_stdout(dir.path(), &["log", "-1", "--format=%s"]),
        "on feature"
    );
}

#[test]
fn revert_commit_undoes_a_commits_changes_as_a_new_commit() {
    let dir = repository();
    fs::write(dir.path().join("file.txt"), b"second\n").unwrap();
    git(dir.path(), &["commit", "-q", "-a", "-m", "second"]);
    let sha = git_stdout(dir.path(), &["rev-parse", "HEAD"]);
    write::apply_at(dir.path(), &[], &GitWriteOp::RevertCommit { sha }).unwrap();
    assert_eq!(fs::read(dir.path().join("file.txt")).unwrap(), b"hello\n");
    assert_eq!(
        git_stdout(dir.path(), &["log", "--format=%s"]),
        "Revert \"second\"\nsecond\nfirst"
    );
}

#[test]
fn reset_hard_moves_head_and_the_worktree() {
    let dir = repository();
    let first = git_stdout(dir.path(), &["rev-parse", "HEAD"]);
    fs::write(dir.path().join("file.txt"), b"second\n").unwrap();
    git(dir.path(), &["commit", "-q", "-a", "-m", "second"]);
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Reset {
            sha: first,
            mode: ubiq_proto::git::GitResetMode::Hard,
        },
    )
    .unwrap();
    assert_eq!(git_stdout(dir.path(), &["log", "--format=%s"]), "first");
    assert_eq!(fs::read(dir.path().join("file.txt")).unwrap(), b"hello\n");
}

#[test]
fn reset_soft_moves_head_and_keeps_the_index() {
    let dir = repository();
    let first = git_stdout(dir.path(), &["rev-parse", "HEAD"]);
    fs::write(dir.path().join("file.txt"), b"second\n").unwrap();
    git(dir.path(), &["commit", "-q", "-a", "-m", "second"]);
    write::apply_at(
        dir.path(),
        &[],
        &GitWriteOp::Reset {
            sha: first,
            mode: ubiq_proto::git::GitResetMode::Soft,
        },
    )
    .unwrap();
    assert_eq!(git_stdout(dir.path(), &["log", "--format=%s"]), "first");
    assert_eq!(
        git_stdout(dir.path(), &["diff", "--cached", "--name-only"]),
        "file.txt"
    );
}

#[test]
fn fetch_all_speaks_ssh() {
    let dir = repository();
    git(
        dir.path(),
        &[
            "remote",
            "add",
            "origin",
            "ssh://127.0.0.1:1/mdnmdn/ubiq.git",
        ],
    );
    let error = write::apply_at(dir.path(), &[], &GitWriteOp::FetchAll).unwrap_err();
    match error {
        GitError::Denied => {}
        GitError::Failed(reason) => assert!(
            !reason
                .to_ascii_lowercase()
                .contains("unsupported url protocol"),
            "ssh remotes must be a supported protocol, got {reason}"
        ),
        other => panic!("expected a transport failure, got {other:?}"),
    }
}

#[test]
fn fetch_all_updates_the_remote_tracking_ref() {
    let origin = repository();
    let local = TempDir::new().unwrap();
    git(
        local.path(),
        &["clone", "-q", origin.path().to_str().unwrap(), "."],
    );
    identity(local.path());
    fs::write(origin.path().join("file.txt"), b"from origin\n").unwrap();
    git(origin.path(), &["commit", "-q", "-am", "origin moved"]);

    write::apply_at(local.path(), &[], &GitWriteOp::FetchAll).unwrap();

    let overview = observe(local.path(), 1, false, &[])
        .unwrap()
        .overview
        .expect("a repository");
    assert_eq!(overview.behind, Some(1));
}

#[test]
fn pull_fast_forwards_to_the_upstream() {
    let origin = repository();
    let local = TempDir::new().unwrap();
    git(
        local.path(),
        &["clone", "-q", origin.path().to_str().unwrap(), "."],
    );
    identity(local.path());
    fs::write(origin.path().join("file.txt"), b"from origin\n").unwrap();
    git(origin.path(), &["commit", "-q", "-am", "origin moved"]);

    write::apply_at(local.path(), &[], &GitWriteOp::Pull).unwrap();

    let body = fs::read_to_string(local.path().join("file.txt")).unwrap();
    assert_eq!(body, "from origin\n");
    let found = observe(local.path(), 1, true, &[]).unwrap();
    assert!(
        found
            .tree
            .map(|tree| tree.entries.is_empty())
            .unwrap_or(true)
    );
}

#[test]
fn push_sends_the_current_branch() {
    let origin = TempDir::new().unwrap();
    git(origin.path(), &["init", "-q", "-b", "main", "--bare"]);
    let local = repository();
    git(
        local.path(),
        &["remote", "add", "origin", origin.path().to_str().unwrap()],
    );
    git(local.path(), &["push", "-q", "-u", "origin", "main"]);
    fs::write(local.path().join("file.txt"), b"pushed\n").unwrap();
    write::apply_at(
        local.path(),
        &[],
        &GitWriteOp::Stage {
            rel_path: "file.txt".into(),
        },
    )
    .unwrap();
    write::apply_at(
        local.path(),
        &[],
        &GitWriteOp::Commit {
            message: "to push".into(),
            amend: false,
        },
    )
    .unwrap();
    write::apply_at(local.path(), &[], &GitWriteOp::Push).unwrap();

    let mirror = TempDir::new().unwrap();
    git(
        mirror.path(),
        &["clone", "-q", origin.path().to_str().unwrap(), "."],
    );
    let body = fs::read_to_string(mirror.path().join("file.txt")).unwrap();
    assert_eq!(body, "pushed\n");
}

// ── a request that names a nested repository ────────────────────────────────

/// Run one request through the worker and collect what it answered, repository by repository.
fn ask(
    root: &Path,
    managed_repos: &[&str],
    repo: &str,
    request: ubiq_host::git::Request,
    replies: usize,
) -> Vec<ubiq_proto::messages::Message> {
    let (hub, host) = ubiq_proto::bus::hub();
    let client = hub.connect();
    let worker = ubiq_host::git::Git::start();
    worker.submit(ubiq_host::git::Job {
        project_id: ubiq_proto::ids::ProjectId::generate(),
        root: root.to_path_buf(),
        managed_repos: managed(managed_repos),
        repo: repo.to_string(),
        request,
        reply_to: host.mailbox(ubiq_proto::bus::To::Client(client.id())),
    });
    (0..replies)
        .map(|_| {
            client
                .from_host()
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("the worker answered")
        })
        .collect()
}

#[test]
fn a_nested_repository_answers_its_own_overview() {
    use ubiq_proto::messages::Message;
    let dir = with_a_nested_clone();
    git(dir.path(), &["checkout", "-q", "-b", "outer-branch"]);

    let inner = ask(
        dir.path(),
        &["inner"],
        "inner",
        ubiq_host::git::Request::Overview,
        1,
    );
    let Message::GitOverview { repo, overview, .. } = &inner[0] else {
        panic!("not an overview: {:?}", inner[0]);
    };
    assert_eq!(repo, "inner");
    let overview = overview.as_ref().expect("the nested repository");
    assert_eq!(overview.head, GitHead::Branch("main".into()));
    assert_eq!(overview.scoped_to, "");

    let outer = ask(
        dir.path(),
        &["inner"],
        "",
        ubiq_host::git::Request::Overview,
        1,
    );
    let Message::GitOverview { repo, overview, .. } = &outer[0] else {
        panic!("not an overview: {:?}", outer[0]);
    };
    assert_eq!(repo, "");
    assert_eq!(
        overview.as_ref().unwrap().head,
        GitHead::Branch("outer-branch".into())
    );
}

#[test]
fn a_nested_repository_answers_its_own_log() {
    use ubiq_proto::messages::Message;
    let dir = with_a_nested_clone();
    let log = |repo: &str| {
        let answered = ask(
            dir.path(),
            &["inner"],
            repo,
            ubiq_host::git::Request::Log {
                cursor: None,
                count: 10,
                rel_path: None,
                first_parent: false,
                rev: None,
            },
            1,
        );
        match answered.into_iter().next().unwrap() {
            Message::GitLogPage {
                repo: echoed,
                commits,
                ..
            } => {
                assert_eq!(echoed, repo);
                commits
                    .into_iter()
                    .map(|commit| commit.summary)
                    .collect::<Vec<_>>()
            }
            other => panic!("not a log page: {other:?}"),
        }
    };
    assert_eq!(log("inner"), vec!["inner first".to_string()]);
    assert_eq!(log(""), vec!["first".to_string()]);
}

#[test]
fn a_repository_the_project_does_not_manage_is_not_found() {
    use ubiq_proto::messages::Message;
    let dir = with_a_nested_clone();
    let answered = ask(
        dir.path(),
        &[],
        "inner",
        ubiq_host::git::Request::Overview,
        1,
    );
    assert!(matches!(
        &answered[0],
        Message::GitError { repo, error: GitError::NotFound, .. } if repo == "inner"
    ));
}

#[test]
fn a_write_to_a_nested_repository_stages_there_and_refreshes_the_project() {
    use ubiq_proto::messages::Message;
    let dir = with_a_nested_clone();
    let answered = ask(
        dir.path(),
        &["inner"],
        "inner",
        ubiq_host::git::Request::Write {
            op: GitWriteOp::Stage {
                rel_path: "inner/kept.txt".into(),
            },
        },
        3,
    );
    assert!(matches!(&answered[0], Message::GitOverview { repo, .. } if repo == "inner"));
    assert!(matches!(&answered[1], Message::GitOverview { repo, .. } if repo.is_empty()));
    let Message::GitWorkingTree { entries, .. } = &answered[2] else {
        panic!("not a working tree: {:?}", answered[2]);
    };
    let staged = entries
        .iter()
        .find(|entry| entry.rel_path == "inner/kept.txt")
        .expect("the nested file is in the merged map");
    assert!(staged.index.is_some());
    assert_eq!(
        git_stdout(
            &dir.path().join("inner"),
            &["diff", "--cached", "--name-only"]
        ),
        "kept.txt"
    );
}
