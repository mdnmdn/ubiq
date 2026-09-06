//! The search worker, against real files: no coordinator, no pane, just a `Job` and a `Mailbox`
//! addressed at a plain bus client.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use tempfile::TempDir;
use ubiq_host::index::text::{self, Text};
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

/// Build a full-text index over every file under `root`, ready to be consulted.
///
/// The `TempDir` holding the index is returned with it: dropping it would delete the directory the
/// reader has mapped.
fn indexed(root: &Path) -> (TempDir, Text) {
    let home = TempDir::new().unwrap();
    let mut index = Text::open(home.path()).unwrap();
    for entry in ignore::WalkBuilder::new(root).build().flatten() {
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let len = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        if !text::indexable(entry.path(), len) {
            continue;
        }
        let Ok(body) = fs::read_to_string(entry.path()) else {
            continue;
        };
        let rel = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .into_owned();
        index.put(&rel, Some(&body));
    }
    index.commit().unwrap();
    index.set_ready(true);
    (home, index)
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
        index: None,
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

// ── The index path ──────────────────────────────────────────────
//
// The index chooses which files are read and nothing else (`D75`), so every assertion here is
// about the two paths agreeing rather than about the index having its own behaviour.

/// The whole correctness claim: the same query over the same tree answers identically whether the
/// walk or the index chose the files.
#[test]
fn an_indexed_search_answers_exactly_what_the_walk_answers() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "needle\nnot this\nneedle again\n").unwrap();
    fs::write(dir.path().join("b.txt"), "a needle in here\n").unwrap();
    fs::write(dir.path().join("c.txt"), "nothing at all\n").unwrap();
    fs::create_dir(dir.path().join("sub")).unwrap();
    fs::write(dir.path().join("sub/d.txt"), "needles, plural\n").unwrap();

    let (walked, walked_client) = job(dir.path(), query("needle"));
    worker::run(&walked);
    let by_walk = drain(&walked_client, &walked);

    let (_home, index) = indexed(&dir.path().canonicalize().unwrap());
    let (mut indexed_job, indexed_client) = job(dir.path(), query("needle"));
    indexed_job.index = Some(index.reader());
    worker::run(&indexed_job);
    let by_index = drain(&indexed_client, &indexed_job);

    let sorted = |answers: &Answers| {
        let mut hits: Vec<(String, Vec<(u32, String)>)> = answers
            .hits
            .iter()
            .map(|hit| {
                (
                    hit.rel_path.clone(),
                    hit.lines
                        .iter()
                        .map(|line| (line.line, line.text.clone()))
                        .collect(),
                )
            })
            .collect();
        hits.sort();
        hits
    };

    assert_eq!(
        sorted(&by_index),
        sorted(&by_walk),
        "hits must be identical"
    );
    assert_eq!(by_index.truncated, by_walk.truncated);
    // Four files hold `needle` as a substring; `c.txt` holds none, so the index never reads it.
    // That difference is the entire point, and it is the one thing that may differ.
    assert_eq!(sorted(&by_walk).len(), 3);
}

/// A query the index cannot bound falls through to the walk, and still finds everything.
#[test]
fn a_regex_query_ignores_the_index_and_still_answers() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "needle\n").unwrap();

    let (_home, index) = indexed(&dir.path().canonicalize().unwrap());
    let (mut job, client) = job(
        dir.path(),
        Query {
            text: "n[e]+dle".to_string(),
            case_sensitive: false,
            whole_word: false,
            regex: true,
        },
    );
    job.index = Some(index.reader());
    worker::run(&job);
    let answers = drain(&client, &job);

    assert_eq!(answers.hits.len(), 1, "a regex must still be answered");
}

/// A query too short to have a trigram takes the walk rather than answering nothing.
#[test]
fn a_two_character_query_walks_rather_than_asking_the_index() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "ab\n").unwrap();

    let (_home, index) = indexed(&dir.path().canonicalize().unwrap());
    let (mut job, client) = job(dir.path(), query("ab"));
    job.index = Some(index.reader());
    worker::run(&job);
    let answers = drain(&client, &job);

    assert_eq!(answers.hits.len(), 1);
}

/// An index whose build has not finished is not consulted.
#[test]
fn an_index_that_is_not_ready_is_not_consulted() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "needle\n").unwrap();

    let (_home, index) = indexed(&dir.path().canonicalize().unwrap());
    // Nothing has been indexed as far as a searcher is concerned.
    index.set_ready(false);

    let (mut job, client) = job(dir.path(), query("needle"));
    job.index = Some(index.reader());
    worker::run(&job);
    let answers = drain(&client, &job);

    assert_eq!(answers.hits.len(), 1, "the walk must still answer");
}

/// A stale index cannot invent a hit: a candidate that no longer matches contributes nothing.
#[test]
fn a_stale_candidate_contributes_no_hits() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "needle\n").unwrap();
    let (_home, index) = indexed(&dir.path().canonicalize().unwrap());

    // The file changes under the index, which still names it as a candidate.
    fs::write(dir.path().join("a.txt"), "haystack\n").unwrap();

    let (mut job, client) = job(dir.path(), query("needle"));
    job.index = Some(index.reader());
    worker::run(&job);
    let answers = drain(&client, &job);

    assert!(
        answers.hits.is_empty(),
        "a candidate is re-read, so a stale entry costs a read and never a wrong hit"
    );
}

/// The excludes still apply when the index chose the files, not only when the walk did.
#[test]
fn an_exclude_removes_an_indexed_candidate() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("keep.txt"), "needle\n").unwrap();
    fs::create_dir(dir.path().join("vendor")).unwrap();
    fs::write(dir.path().join("vendor/skip.txt"), "needle\n").unwrap();

    let (_home, index) = indexed(&dir.path().canonicalize().unwrap());
    let (mut job, client) = job_with(
        dir.path(),
        query("needle"),
        Filter::default(),
        vec!["vendor".to_string()],
        Vec::new(),
    );
    job.index = Some(index.reader());
    worker::run(&job);
    let answers = drain(&client, &job);

    let paths: Vec<&str> = answers
        .hits
        .iter()
        .map(|hit| hit.rel_path.as_str())
        .collect();
    assert_eq!(paths, vec!["keep.txt"]);
}

/// A query the regex engine rejects still reaches the external fallback with a warm index present.
///
/// The regression this pins: the index gate sits *after* the matcher is built, so it can never
/// take the fallback's branch away from a query that needs it.
#[test]
fn a_bad_regex_still_reaches_the_fallback_with_an_index() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "needle\n").unwrap();

    let (_home, index) = indexed(&dir.path().canonicalize().unwrap());
    let (mut job, client) = job_with(
        dir.path(),
        Query {
            // A backreference: ripgrep's engine refuses it, a PCRE-ish tool accepts it.
            text: r"(needle)\1".to_string(),
            case_sensitive: false,
            whole_word: false,
            regex: true,
        },
        Filter::default(),
        Vec::new(),
        // No tool configured, so the fallback declines and the error is answered — which is
        // reached only if the fallback branch ran at all.
        Vec::new(),
    );
    job.index = Some(index.reader());
    worker::run(&job);

    let message = client
        .from_host()
        .recv_timeout(PATIENCE)
        .expect("an answer");
    assert!(
        matches!(
            message,
            Message::SearchError {
                error: SearchError::BadQuery(_),
                ..
            }
        ),
        "expected the bad-query path, got {message:?}"
    );
}
