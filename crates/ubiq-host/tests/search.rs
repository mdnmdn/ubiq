//! The search worker, against real files: no coordinator, no pane, just a `Job` and a `Mailbox`
//! addressed at a plain bus client.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use tempfile::TempDir;
use ubiq_host::search::{Job, ceiling, fallback, worker};
use ubiq_proto::bus::{self, To};
use ubiq_proto::ids::{ProjectId, SearchId};
use ubiq_proto::messages::Message;
use ubiq_proto::search::{Batch, FileHit, Filter, Query, SearchError};

/// Long enough for a search over a handful of small files on a loaded machine.
const PATIENCE: Duration = Duration::from_secs(5);

/// A project with a plain top-level file and a subfolder, plus whatever the test adds.
fn project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("top.txt"), "needle\n").unwrap();
    fs::create_dir(dir.path().join("sub")).unwrap();
    fs::write(dir.path().join("sub/inner.txt"), "nothing here\n").unwrap();
    dir
}

/// A literal, case-insensitive query for `text` — the default a searcher UI would send.
fn query(text: &str) -> Query {
    Query {
        text: text.to_string(),
        case_sensitive: false,
        whole_word: false,
        regex: false,
    }
}

/// One search job over `root`, answering on a fresh bus client. The client is returned alongside
/// so its inbox can be drained — dropping it would tear down the mailbox's destination.
fn job(root: &Path, query: Query) -> (Job, bus::Client) {
    job_with(root, query, Filter::default(), Vec::new(), Vec::new())
}

/// The full shape, for the filter, exclude and fallback tests.
fn job_with(
    root: &Path,
    query: Query,
    filter: Filter,
    excludes: Vec<String>,
    fallbacks: Vec<String>,
) -> (Job, bus::Client) {
    let (hub, host) = bus::hub();
    let client = hub.connect();
    let reply_to = host.mailbox(To::Client(client.id()));
    let job = Job {
        project_id: ProjectId::generate(),
        search_id: SearchId::generate(),
        // Production roots come from `ProjectRecord.path`, already canonical; the temp dir here
        // is not (`/var` → `/private/var` on macOS), and subdirs resolve through
        // `files::path::resolve`, which canonicalizes.
        root: root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
        query,
        filter,
        excludes,
        fallbacks,
        cancel: Arc::new(AtomicBool::new(false)),
        reply_to,
    };
    (job, client)
}

/// Everything one search said, in order, up to and including `SearchFinished`.
struct Answers {
    hits: Vec<FileHit>,
    /// `files_seen` from every `SearchProgress`, in the order they arrived.
    progress: Vec<usize>,
    truncated: bool,
}

/// Drain `client`'s inbox until `SearchFinished`, checking every message is addressed to `job`.
fn drain(client: &bus::Client, job: &Job) -> Answers {
    let mut hits = Vec::new();
    let mut progress = Vec::new();

    loop {
        let message = client
            .from_host()
            .recv_timeout(PATIENCE)
            .expect("the worker to answer before the search finished");

        match message {
            Message::SearchMatches {
                project_id,
                search_id,
                batch,
            } => {
                assert_eq!(project_id, job.project_id);
                assert_eq!(search_id, job.search_id);
                match batch {
                    Batch::Files(files) => hits.extend(files),
                    Batch::Tasks(_) => panic!("v1 searches files only"),
                }
            }
            Message::SearchProgress {
                project_id,
                search_id,
                files_seen,
            } => {
                assert_eq!(project_id, job.project_id);
                assert_eq!(search_id, job.search_id);
                progress.push(files_seen);
            }
            Message::SearchFinished {
                project_id,
                search_id,
                truncated,
                ..
            } => {
                assert_eq!(project_id, job.project_id);
                assert_eq!(search_id, job.search_id);
                return Answers {
                    hits,
                    progress,
                    truncated,
                };
            }
            other => panic!("unexpected message from the search worker: {other:?}"),
        }
    }
}

#[test]
fn hits_are_found_and_arrive_batched() {
    let dir = TempDir::new().unwrap();
    // More than `BATCH_FILES` files with one hit each, so the batch has to flush more than once.
    for i in 0..(ceiling::BATCH_FILES + 6) {
        fs::write(dir.path().join(format!("hit-{i}.txt")), "needle\n").unwrap();
    }

    let (job, client) = job(dir.path(), query("needle"));
    worker::run(&job);
    let answers = drain(&client, &job);

    assert_eq!(answers.hits.len(), ceiling::BATCH_FILES + 6);
    assert!(answers.hits.iter().all(|hit| hit.lines.len() == 1));
    assert!(!answers.truncated);
}

#[test]
fn more_than_hits_per_file_sets_file_hit_truncated() {
    let dir = project();
    let body = "needle\n".repeat(ceiling::HITS_PER_FILE + 50);
    fs::write(dir.path().join("busy.txt"), body).unwrap();

    let (job, client) = job(dir.path(), query("needle"));
    worker::run(&job);
    let answers = drain(&client, &job);

    let busy = answers
        .hits
        .iter()
        .find(|hit| hit.rel_path == "busy.txt")
        .expect("busy.txt should have hits");
    assert_eq!(busy.lines.len(), ceiling::HITS_PER_FILE);
    assert!(busy.truncated);
}

#[test]
fn a_thousand_and_one_files_stop_the_walk_and_set_search_finished_truncated() {
    let dir = TempDir::new().unwrap();
    for i in 0..(ceiling::FILES_WITH_HITS + 1) {
        // One byte of content is enough to hit; keep the fixture cheap.
        fs::write(dir.path().join(format!("f{i}.txt")), "x").unwrap();
    }

    let (job, client) = job(dir.path(), query("x"));
    worker::run(&job);
    let answers = drain(&client, &job);

    // Bug 1's regression: the ceiling used to never fire, so `truncated` was always false here.
    assert!(answers.truncated);
    assert!(answers.hits.len() <= ceiling::FILES_WITH_HITS + 1);
}

#[test]
fn files_seen_counts_files_without_hits_too() {
    let dir = TempDir::new().unwrap();
    for i in 0..5 {
        fs::write(dir.path().join(format!("miss-{i}.txt")), "nothing\n").unwrap();
    }
    fs::write(dir.path().join("hit.txt"), "needle\n").unwrap();

    let (job, client) = job(dir.path(), query("needle"));
    worker::run(&job);
    let answers = drain(&client, &job);

    assert_eq!(answers.hits.len(), 1);
    // Bug 2's regression: `files_seen` used to count only files with hits, so this would read 1.
    assert_eq!(*answers.progress.last().unwrap(), 6);
}

#[test]
fn a_search_that_finds_nothing_still_reports_progress_and_finishes() {
    let dir = project();

    let (job, client) = job(dir.path(), query("no-such-text-anywhere"));
    worker::run(&job);
    let answers = drain(&client, &job);

    assert!(answers.hits.is_empty());
    assert!(!answers.truncated);
    // The final progress report fires even when no batch ever did.
    assert_eq!(*answers.progress.last().unwrap(), 2);
}

#[test]
fn a_preset_cancel_flag_still_answers_search_finished() {
    let dir = project();
    let (job, client) = job(dir.path(), query("needle"));
    job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);

    worker::run(&job);
    let answers = drain(&client, &job);

    assert!(answers.hits.is_empty());
    assert!(!answers.truncated);
}

#[test]
fn a_gitignored_file_is_skipped() {
    let dir = project();
    // `ignore` only honours `.gitignore` inside an actual repository.
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
    fs::write(dir.path().join("ignored.txt"), "needle\n").unwrap();
    fs::write(dir.path().join("kept.txt"), "needle\n").unwrap();

    let (job, client) = job(dir.path(), query("needle"));
    worker::run(&job);
    let answers = drain(&client, &job);

    let paths: Vec<&str> = answers
        .hits
        .iter()
        .map(|hit| hit.rel_path.as_str())
        .collect();
    assert!(paths.contains(&"kept.txt"));
    assert!(!paths.contains(&"ignored.txt"));
}

/// The one message a job that never gets past filter or query validation sends.
fn expect_error(client: &bus::Client, job: &Job) -> SearchError {
    match client
        .from_host()
        .recv_timeout(PATIENCE)
        .expect("the worker to answer")
    {
        Message::SearchError {
            project_id,
            search_id,
            error,
        } => {
            assert_eq!(project_id, job.project_id);
            assert_eq!(search_id, job.search_id);
            error
        }
        other => panic!("expected a SearchError, got {other:?}"),
    }
}

#[test]
fn a_pattern_filter_limits_which_files_are_searched() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("keep.rs"), "needle\n").unwrap();
    fs::write(dir.path().join("skip.txt"), "needle\n").unwrap();

    let filter = Filter {
        patterns: vec!["*.rs".to_string()],
        subdir: None,
    };
    let (job, client) = job_with(dir.path(), query("needle"), filter, Vec::new(), Vec::new());
    worker::run(&job);
    let answers = drain(&client, &job);

    let paths: Vec<&str> = answers
        .hits
        .iter()
        .map(|hit| hit.rel_path.as_str())
        .collect();
    assert_eq!(paths, vec!["keep.rs"]);
}

#[test]
fn a_subdir_scopes_the_walk_and_hits_stay_project_relative() {
    let dir = project();
    fs::write(dir.path().join("sub/needle.txt"), "needle\n").unwrap();

    let filter = Filter {
        patterns: Vec::new(),
        subdir: Some("sub".to_string()),
    };
    let (job, client) = job_with(dir.path(), query("needle"), filter, Vec::new(), Vec::new());
    worker::run(&job);
    let answers = drain(&client, &job);

    let paths: Vec<&str> = answers
        .hits
        .iter()
        .map(|hit| hit.rel_path.as_str())
        .collect();
    // Project-relative, not relative to the subdir the walk actually started at.
    assert_eq!(paths, vec!["sub/needle.txt"]);
    assert!(!paths.iter().any(|p| p.contains("top.txt")));
}

#[test]
fn a_subdir_that_leaves_the_project_is_a_bad_filter() {
    let dir = project();
    let filter = Filter {
        patterns: Vec::new(),
        subdir: Some("../".to_string()),
    };
    let (job, client) = job_with(dir.path(), query("needle"), filter, Vec::new(), Vec::new());
    worker::run(&job);
    assert!(matches!(
        expect_error(&client, &job),
        SearchError::BadFilter(_)
    ));
}

#[test]
fn an_absolute_subdir_is_a_bad_filter() {
    let dir = project();
    let filter = Filter {
        patterns: Vec::new(),
        subdir: Some("/etc".to_string()),
    };
    let (job, client) = job_with(dir.path(), query("needle"), filter, Vec::new(), Vec::new());
    worker::run(&job);
    assert!(matches!(
        expect_error(&client, &job),
        SearchError::BadFilter(_)
    ));
}

#[test]
fn a_subdir_that_is_a_file_not_a_directory_is_a_bad_filter() {
    let dir = project();
    let filter = Filter {
        patterns: Vec::new(),
        subdir: Some("top.txt".to_string()),
    };
    let (job, client) = job_with(dir.path(), query("needle"), filter, Vec::new(), Vec::new());
    worker::run(&job);
    assert!(matches!(
        expect_error(&client, &job),
        SearchError::BadFilter(_)
    ));
}

#[test]
fn a_global_exclude_removes_a_file_that_would_otherwise_hit() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("excluded.txt"), "needle\n").unwrap();
    fs::write(dir.path().join("kept.txt"), "needle\n").unwrap();

    let (job, client) = job_with(
        dir.path(),
        query("needle"),
        Filter::default(),
        vec!["excluded.txt".to_string()],
        Vec::new(),
    );
    worker::run(&job);
    let answers = drain(&client, &job);

    let paths: Vec<&str> = answers
        .hits
        .iter()
        .map(|hit| hit.rel_path.as_str())
        .collect();
    assert_eq!(paths, vec!["kept.txt"]);
}

#[test]
fn a_per_project_exclude_removes_a_file_that_would_otherwise_hit() {
    // The coordinator merges the project's own excludes into `Job.excludes` before submitting —
    // from the worker's side that is indistinguishable from a global one, so this exercises the
    // same code path with a different origin for the string.
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("secret.env"), "needle\n").unwrap();
    fs::write(dir.path().join("kept.txt"), "needle\n").unwrap();

    let (job, client) = job_with(
        dir.path(),
        query("needle"),
        Filter::default(),
        vec!["*.env".to_string()],
        Vec::new(),
    );
    worker::run(&job);
    let answers = drain(&client, &job);

    let paths: Vec<&str> = answers
        .hits
        .iter()
        .map(|hit| hit.rel_path.as_str())
        .collect();
    assert_eq!(paths, vec!["kept.txt"]);
}

#[test]
fn an_unparseable_glob_is_a_bad_filter() {
    let dir = project();
    let filter = Filter {
        patterns: vec!["[".to_string()],
        subdir: None,
    };
    let (job, client) = job_with(dir.path(), query("needle"), filter, Vec::new(), Vec::new());
    worker::run(&job);
    assert!(matches!(
        expect_error(&client, &job),
        SearchError::BadFilter(_)
    ));
}

#[test]
fn with_no_fallbacks_configured_a_bad_regex_still_answers_bad_query() {
    let dir = project();
    let mut bad_regex = query("(");
    // An unclosed group is invalid regex syntax to any engine, not merely unsupported syntax —
    // proving the query never compiled rather than that a fallback silently swallowed the error.
    bad_regex.regex = true;

    let (job, client) = job_with(
        dir.path(),
        bad_regex,
        Filter::default(),
        Vec::new(),
        Vec::new(),
    );
    worker::run(&job);
    assert!(matches!(
        expect_error(&client, &job),
        SearchError::BadQuery(_)
    ));
}

/// `fallback::pick` never chooses a tool that is not actually on this machine — proven against a
/// name nothing installs, so the test needs no real `ag` or `grep` on the runner.
#[test]
fn fallback_pick_finds_nothing_for_a_tool_name_that_does_not_exist() {
    assert!(fallback::pick(&["definitely-not-a-real-search-tool".to_string()]).is_none());
}
