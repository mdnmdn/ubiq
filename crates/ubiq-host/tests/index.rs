//! The index worker, against real files: no coordinator and no watcher, just jobs on its queue.
//!
//! What is asserted here is the worker's own behaviour — that a build fills, that a change lands,
//! that a truncated burst rebuilds, that a drop forgets. Whether a candidate becomes a *hit* is
//! `tests/search.rs`, because the index never produces one (`D75`).

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use ubiq_host::index::{Index, Job};
use ubiq_proto::ids::ProjectId;

/// Long enough for a build over a handful of small files on a loaded machine.
const PATIENCE: Duration = Duration::from_secs(5);

/// A project with two files, one holding the needle.
fn project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "needle\n").unwrap();
    fs::write(dir.path().join("b.txt"), "nothing here\n").unwrap();
    dir
}

/// Wait for the worker to publish a reader for `project_id`.
fn wait_for_reader(index: &Index, project_id: ProjectId) -> ubiq_host::index::text::Reader {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(reader) = index.reader(project_id).filter(|reader| reader.ready()) {
            return reader;
        }
        assert!(Instant::now() < deadline, "the index was never built");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Wait until `needle` resolves to exactly `want`, or give up.
///
/// Polling rather than a signal: a commit is debounced on purpose, so the worker deliberately does
/// not answer the instant it is told something.
fn wait_for_candidates(reader: &ubiq_host::index::text::Reader, needle: &str, want: &[&str]) {
    let deadline = Instant::now() + PATIENCE;
    let mut last = Vec::new();
    loop {
        if let Ok(Some((mut paths, _))) = reader.candidates(needle) {
            paths.sort();
            if paths == want {
                return;
            }
            last = paths;
        }
        if Instant::now() >= deadline {
            panic!("expected candidates {want:?} for {needle:?}, last saw {last:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn build(index: &Index, project_id: ProjectId, root: &Path, home: &Path) {
    index.submit(Job::Build {
        project_id,
        root: root.to_path_buf(),
        home: home.to_path_buf(),
        excludes: Vec::new(),
    });
}

#[test]
fn a_build_fills_the_index_and_publishes_a_reader() {
    let dir = project();
    let home = TempDir::new().unwrap();
    let index = Index::start();
    let project_id = ProjectId::generate();

    // Before the build there is no reader at all, which is what makes a search walk.
    assert!(index.reader(project_id).is_none());

    build(&index, project_id, dir.path(), home.path());
    let reader = wait_for_reader(&index, project_id);

    wait_for_candidates(&reader, "needle", &["a.txt"]);
}

#[test]
fn a_gitignored_file_is_never_indexed() {
    let dir = project();
    // `ignore` only honours `.gitignore` inside an actual repository.
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join(".gitignore"), "secret.txt\n").unwrap();
    fs::write(dir.path().join("secret.txt"), "needle\n").unwrap();

    let home = TempDir::new().unwrap();
    let index = Index::start();
    let project_id = ProjectId::generate();
    build(&index, project_id, dir.path(), home.path());
    let reader = wait_for_reader(&index, project_id);

    // The index runs the same walk content search does, so the two can never disagree about what
    // a project's files are.
    wait_for_candidates(&reader, "needle", &["a.txt"]);
}

#[test]
fn a_changed_file_is_reindexed_and_a_deleted_one_is_dropped() {
    let dir = project();
    let home = TempDir::new().unwrap();
    let index = Index::start();
    let project_id = ProjectId::generate();
    build(&index, project_id, dir.path(), home.path());
    let reader = wait_for_reader(&index, project_id);
    wait_for_candidates(&reader, "needle", &["a.txt"]);

    // A file that gains the needle is found once its change is reported.
    fs::write(dir.path().join("b.txt"), "needle too\n").unwrap();
    index.submit(Job::Changed {
        project_id,
        root: dir.path().to_path_buf(),
        paths: vec!["b.txt".to_string()],
        truncated: false,
    });
    wait_for_candidates(&reader, "needle", &["a.txt", "b.txt"]);

    // A file that goes away stops being a candidate — the same delete-then-add path.
    fs::remove_file(dir.path().join("a.txt")).unwrap();
    index.submit(Job::Changed {
        project_id,
        root: dir.path().to_path_buf(),
        paths: vec!["a.txt".to_string()],
        truncated: false,
    });
    wait_for_candidates(&reader, "needle", &["b.txt"]);
}

#[test]
fn a_truncated_burst_rebuilds_from_the_tree() {
    let dir = project();
    let home = TempDir::new().unwrap();
    let index = Index::start();
    let project_id = ProjectId::generate();
    build(&index, project_id, dir.path(), home.path());
    let reader = wait_for_reader(&index, project_id);
    wait_for_candidates(&reader, "needle", &["a.txt"]);

    // Changes the watcher could not name: it dropped the list and said so.
    fs::write(dir.path().join("c.txt"), "needle three\n").unwrap();
    fs::remove_file(dir.path().join("a.txt")).unwrap();
    index.submit(Job::Changed {
        project_id,
        root: dir.path().to_path_buf(),
        paths: Vec::new(),
        truncated: true,
    });

    // The rebuild reads the tree rather than the burst, so it sees both changes.
    wait_for_candidates(&reader, "needle", &["c.txt"]);
}

#[test]
fn a_dropped_project_has_no_reader() {
    let dir = project();
    let home = TempDir::new().unwrap();
    let index = Index::start();
    let project_id = ProjectId::generate();
    build(&index, project_id, dir.path(), home.path());
    wait_for_reader(&index, project_id);

    index.submit(Job::Drop(project_id));

    let deadline = Instant::now() + PATIENCE;
    while index.reader(project_id).is_some() {
        assert!(Instant::now() < deadline, "the reader was never withdrawn");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_change_for_a_project_with_no_index_is_ignored() {
    let dir = project();
    let index = Index::start();
    let project_id = ProjectId::generate();

    // No build was ever asked for. A watcher may still be reporting — a level of `none` keeps the
    // watch and drops the index — and that must not resurrect one.
    index.submit(Job::Changed {
        project_id,
        root: dir.path().to_path_buf(),
        paths: vec!["a.txt".to_string()],
        truncated: false,
    });

    std::thread::sleep(Duration::from_millis(200));
    assert!(index.reader(project_id).is_none());
}

#[test]
fn a_second_build_keeps_the_index_that_is_already_answering() {
    let dir = project();
    let home = TempDir::new().unwrap();
    let index = Index::start();
    let project_id = ProjectId::generate();

    build(&index, project_id, dir.path(), home.path());
    let reader = wait_for_reader(&index, project_id);
    wait_for_candidates(&reader, "needle", &["a.txt"]);

    // A second window opening the same project must not tear down an index mid-answer.
    build(&index, project_id, dir.path(), home.path());
    std::thread::sleep(Duration::from_millis(200));
    assert!(index.reader(project_id).is_some_and(|r| r.ready()));
    wait_for_candidates(&reader, "needle", &["a.txt"]);
}

// ── The level ───────────────────────────────────────────────────
//
// What a level decides is what is *kept*, not what is noticed: the watcher runs at every level,
// including `None`. These pin the transitions the coordinator drives.

#[test]
fn dropping_to_none_and_coming_back_rebuilds() {
    let dir = project();
    let home = TempDir::new().unwrap();
    let index = Index::start();
    let project_id = ProjectId::generate();

    build(&index, project_id, dir.path(), home.path());
    let reader = wait_for_reader(&index, project_id);
    wait_for_candidates(&reader, "needle", &["a.txt"]);

    // The user turns indexing off: what was kept is given back.
    index.submit(Job::Drop(project_id));
    let deadline = Instant::now() + PATIENCE;
    while index.reader(project_id).is_some() {
        assert!(Instant::now() < deadline, "the reader was never withdrawn");
        std::thread::sleep(Duration::from_millis(20));
    }

    // And on again. A build after a drop is a cold build, not a resurrection of the old handle.
    build(&index, project_id, dir.path(), home.path());
    let reader = wait_for_reader(&index, project_id);
    wait_for_candidates(&reader, "needle", &["a.txt"]);
}

#[test]
fn an_index_left_on_disk_does_not_keep_a_file_that_went_away() {
    let dir = project();
    let home = TempDir::new().unwrap();
    let project_id = ProjectId::generate();

    // A first session builds one and goes away.
    {
        let index = Index::start();
        build(&index, project_id, dir.path(), home.path());
        let reader = wait_for_reader(&index, project_id);
        wait_for_candidates(&reader, "needle", &["a.txt"]);
    }

    // The project changes while nothing is watching it.
    fs::remove_file(dir.path().join("a.txt")).unwrap();
    fs::write(dir.path().join("c.txt"), "needle now here\n").unwrap();

    // The next session must not answer with `a.txt`: nothing walks a file that is not there, so
    // an index that merely wrote over what it found would keep it forever as a candidate that
    // cannot exist.
    let index = Index::start();
    build(&index, project_id, dir.path(), home.path());
    let reader = wait_for_reader(&index, project_id);
    wait_for_candidates(&reader, "needle", &["c.txt"]);
}
