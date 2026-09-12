//! A project's repository as the host observes it, against a real git directory.
//!
//! Every test here builds a scratch repository in a temporary directory and drives the host's own
//! `git` — because what is being asserted is that the host agrees with version control, and a
//! fixture written by hand would only assert that it agrees with the fixture.

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use ubiq_host::git::{nested, observe};
use ubiq_proto::git::{GitHead, GitMark, GitPathChange, GitSubmoduleState};

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
