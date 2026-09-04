//! The search worker: walk the project's files, match a pattern, stream batches back.
//!
//! The walk uses `ignore::WalkBuilder` so the project's own `.gitignore` rules are respected.
//! Matching goes through `grep-regex` and `grep-searcher`, which is what ripgrep uses. The worker
//! is interruptible between files via an `Arc<AtomicBool>`.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use grep_matcher::Matcher;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use ubiq_proto::messages::Message;
use ubiq_proto::search::{self, Batch, FileHit, LineHit, Source};

use super::Job;
use super::ceiling;
use super::fallback;
use super::walk;

/// How long an external fallback tool is given before it is killed. See [`fallback::run`].
const FALLBACK_DEADLINE: Duration = Duration::from_secs(10);

/// Run one search job. Sends [`Message::SearchMatches`] batches, periodic
/// [`Message::SearchProgress`], and a final [`Message::SearchFinished`].
pub fn run(job: &Job) {
    let project_id = job.project_id;
    let search_id = job.search_id;
    let started = Instant::now();

    // A search's life is visible at the default level: this line, the one at the end, and the
    // fallback's own three. **Lengths and counts, never payloads** — the query is user text and a
    // hit carries file contents, so neither is logged here at any level.
    tracing::info!(
        search = %search_id,
        project = %project_id,
        root = %job.root.display(),
        query_len = job.query.text.len(),
        regex = job.query.regex,
        patterns = job.filter.patterns.len(),
        narrowed = job.filter.subdir.is_some(),
        excludes = job.excludes.len(),
        "search started"
    );

    // Resolve the filter's starting subdirectory against the project root, through the same
    // boundary the file family uses — `crate::files::path` already validates `..`, absolute
    // paths, symlink escapes and NUL, and is already tested on its own.
    let start = match &job.filter.subdir {
        Some(subdir) => match crate::files::path::resolve(&job.root, subdir) {
            Ok(path) if path.is_dir() => path,
            Ok(_) => {
                job.reply_to.send(Message::SearchError {
                    project_id,
                    search_id,
                    error: search::SearchError::BadFilter(
                        "that path is not a directory".to_string(),
                    ),
                });
                return;
            }
            Err(error) => {
                job.reply_to.send(Message::SearchError {
                    project_id,
                    search_id,
                    error: search::SearchError::BadFilter(error.to_string()),
                });
                return;
            }
        },
        None => job.root.clone(),
    };

    // Build the matcher from the query. A bad pattern tries a configured external tool before
    // answering [`SearchError::BadQuery`] — a fixed regex a user typed for `grep -E` is exactly
    // the case ripgrep's stricter engine rejects and a PCRE-ish tool accepts.
    let matcher = match RegexMatcherBuilder::new()
        .case_insensitive(!job.query.case_sensitive)
        .fixed_strings(!job.query.regex)
        .word(job.query.whole_word)
        .build(&job.query.text)
    {
        Ok(matcher) => Arc::new(matcher),
        Err(error) => {
            if run_fallback(job, &start, &error.to_string()) {
                return;
            }
            job.reply_to.send(Message::SearchError {
                project_id,
                search_id,
                error: search::SearchError::BadQuery(error.to_string()),
            });
            return;
        }
    };

    // Walk the project tree: the project's own ignore rules (`.gitignore`, `.ignore`, hidden
    // files), plus the filter's include globs and every exclude. A glob that will not compile is
    // a [`SearchError::BadFilter`].
    let builder = match walk::builder(&job.root, &start, &job.filter.patterns, &job.excludes) {
        Ok(builder) => builder,
        Err(error) => {
            job.reply_to.send(Message::SearchError {
                project_id,
                search_id,
                error: search::SearchError::BadFilter(error),
            });
            return;
        }
    };
    let walker = builder.build_parallel();

    let cancel = job.cancel.clone();

    // Shared state across the parallel walk. The walk serialises at the file level: each file
    // is visited by exactly one thread, and the accumulator is only touched between files.
    let state = Arc::new(Mutex::new(State::new()));

    walker.run({
        let state = state.clone();
        let cancel = cancel.clone();
        let reply_to = job.reply_to.clone();
        let walk_root = job.root.clone();

        move || {
            let cancel = cancel.clone();
            let matcher = matcher.clone();
            let state = state.clone();
            let reply_to = reply_to.clone();
            let walk_root = walk_root.clone();

            Box::new(move |entry| {
                // Check the cancel flag between files.
                if cancel.load(Ordering::Relaxed) {
                    return ignore::WalkState::Quit;
                }

                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => return ignore::WalkState::Continue,
                };

                // Only files are searchable.
                if !entry.file_type().is_some_and(|t| t.is_file()) {
                    return ignore::WalkState::Continue;
                }

                // Compute the project-relative path by stripping the walk root.
                let abs_path = entry.path();
                let rel_path = abs_path
                    .strip_prefix(&walk_root)
                    .unwrap_or(abs_path)
                    .to_string_lossy()
                    .into_owned();

                // Search the file using the UTF8 sink, which handles binary detection and
                // line reading for us.
                let mut file_hits: Vec<LineHit> = Vec::new();
                let mut searcher = SearcherBuilder::new()
                    .line_number(true)
                    .binary_detection(BinaryDetection::quit(0))
                    .build();

                let sink = UTF8(
                    |line_number: u64, line_text: &str| -> Result<bool, std::io::Error> {
                        let ranges = compute_match_ranges(&matcher, line_text);

                        if !ranges.is_empty() {
                            file_hits.push(LineHit {
                                line: line_number as u32,
                                text: line_text.to_string(),
                                ranges,
                            });
                        }

                        // Stop the searcher early if the per-file ceiling is hit.
                        Ok(file_hits.len() < ceiling::HITS_PER_FILE)
                    },
                );

                let search_result = searcher.search_path(&*matcher, abs_path, sink);
                let _ = search_result;

                // Every visited file passes through this shared tail, hits or not: `files_seen`,
                // progress, batching and the ceiling checks all depend on seeing every file, not
                // just the ones that matched.
                let mut state = state.lock().unwrap();
                state.saw_file();

                if !file_hits.is_empty() {
                    let truncated = file_hits.len() >= ceiling::HITS_PER_FILE;
                    state.add_file(FileHit {
                        rel_path,
                        lines: file_hits,
                        truncated,
                    });
                }

                let batch = state.should_flush().then(|| state.take_batch());
                let files_seen = state.files_seen;
                let report = files_seen - state.reported_at >= ceiling::PROGRESS_INTERVAL;
                if report {
                    state.reported_at = files_seen;
                }
                let total_hits = state.total_hits;
                let files_with_hits = state.files_with_hits;
                // Drop the lock before sending.
                drop(state);

                if let Some(batch) = batch {
                    reply_to.send(Message::SearchMatches {
                        project_id,
                        search_id,
                        batch,
                    });
                }

                // Progress report at intervals, independent of whether a batch flushed — a search
                // with few hits must still be seen to be moving.
                if report {
                    reply_to.send(Message::SearchProgress {
                        project_id,
                        search_id,
                        files_seen,
                    });
                }

                // Check ceilings.
                if total_hits >= ceiling::TOTAL_HITS || files_with_hits >= ceiling::FILES_WITH_HITS
                {
                    cancel.store(true, Ordering::Relaxed);
                    return ignore::WalkState::Quit;
                }

                ignore::WalkState::Continue
            })
        }
    });

    // Flush any remaining hits.
    let mut state = state.lock().unwrap();
    let truncated = state.total_hits >= ceiling::TOTAL_HITS
        || state.files_with_hits >= ceiling::FILES_WITH_HITS;
    let files_seen = state.files_seen;
    let files_with_hits = state.files_with_hits;
    let total_hits = state.total_hits;
    let searched = vec![Source::File];

    let final_batch = (!state.is_empty()).then(|| state.take_batch());
    drop(state);

    if let Some(batch) = final_batch {
        job.reply_to.send(Message::SearchMatches {
            project_id,
            search_id,
            batch,
        });
    }

    // A last progress report carrying the true count, so a search that found nothing still says
    // it looked rather than looking indistinguishable from one that never started.
    job.reply_to.send(Message::SearchProgress {
        project_id,
        search_id,
        files_seen,
    });

    job.reply_to.send(Message::SearchFinished {
        project_id,
        search_id,
        searched,
        truncated,
    });

    tracing::info!(
        search = %search_id,
        project = %project_id,
        files_seen,
        files_with_hits,
        total_hits,
        elapsed_ms = started.elapsed().as_millis(),
        truncated,
        // Set by a cancel, by a supersede, or by a ceiling the walk hit — `truncated` tells the
        // last of those apart from the first two.
        stopped_early = job.cancel.load(Ordering::Relaxed),
        "search finished"
    );

    // The flag now also means "this search is over", cancelled or not — the coordinator reaps
    // `active_searches` entries by reading it, not just a cancel request.
    job.cancel.store(true, Ordering::Relaxed);
}

/// Try an external tool when the built-in regex engine rejected `job.query`. Answers `true` when
/// it resolved the request — successfully or not — so the caller falls through to `BadQuery` only
/// when this answers `false`, which is "no tool configured or installed", exactly as before this
/// existed.
fn run_fallback(job: &Job, start: &Path, reason: &str) -> bool {
    let Some(chosen) = fallback::pick(&job.fallbacks) else {
        return false;
    };
    // Three log lines every time a fallback runs, so it is never invisible: the tool chosen, why
    // it was needed, then the outcome below.
    tracing::info!(
        tool = %chosen.tool,
        program = %chosen.program.display(),
        "external search fallback chosen"
    );
    tracing::info!(tool = %chosen.tool, reason, "the built-in regex engine rejected the query");

    let fallback_started = Instant::now();
    match fallback::run(&chosen, job, start, FALLBACK_DEADLINE) {
        Ok((hits, total_hits)) => {
            tracing::info!(
                tool = %chosen.tool,
                elapsed_ms = fallback_started.elapsed().as_millis(),
                hits = total_hits,
                "fallback search finished"
            );
            let truncated =
                total_hits >= ceiling::TOTAL_HITS || hits.len() >= ceiling::FILES_WITH_HITS;
            if !hits.is_empty() {
                job.reply_to.send(Message::SearchMatches {
                    project_id: job.project_id,
                    search_id: job.search_id,
                    batch: Batch::Files(hits),
                });
            }
            job.reply_to.send(Message::SearchProgress {
                project_id: job.project_id,
                search_id: job.search_id,
                files_seen: total_hits,
            });
            job.reply_to.send(Message::SearchFinished {
                project_id: job.project_id,
                search_id: job.search_id,
                searched: vec![Source::File],
                truncated,
            });
            job.cancel.store(true, Ordering::Relaxed);
            true
        }
        Err(error) => {
            tracing::warn!(tool = %chosen.tool, error, "fallback search failed");
            false
        }
    }
}

/// Worker-level state, protected by a mutex across the parallel walk's file-level serialisation.
struct State {
    batch_files: Vec<FileHit>,
    /// Every file the walk visited, hits or not — what `SearchProgress` reports.
    files_seen: usize,
    /// Files that contributed at least one hit — what `FILES_WITH_HITS` bounds.
    files_with_hits: usize,
    total_hits: usize,
    /// Accumulated hits since last flush.
    pending_hits: usize,
    /// When the current batch started, so a slow trickle is still flushed.
    opened_at: Instant,
    /// Progress last reported at this `files_seen`.
    reported_at: usize,
}

impl State {
    fn new() -> Self {
        Self {
            batch_files: Vec::new(),
            files_seen: 0,
            files_with_hits: 0,
            total_hits: 0,
            pending_hits: 0,
            opened_at: Instant::now(),
            reported_at: 0,
        }
    }

    /// A file the walk visited, whether or not it had hits.
    fn saw_file(&mut self) {
        self.files_seen += 1;
    }

    fn add_file(&mut self, hit: FileHit) {
        self.files_with_hits += 1;
        self.pending_hits += hit.lines.len();
        self.total_hits += hit.lines.len();
        self.batch_files.push(hit);
    }

    fn should_flush(&self) -> bool {
        !self.batch_files.is_empty()
            && (self.batch_files.len() >= ceiling::BATCH_FILES
                || self.pending_hits >= ceiling::BATCH_HITS
                || self.opened_at.elapsed() >= ceiling::BATCH_INTERVAL)
    }

    fn take_batch(&mut self) -> Batch {
        self.pending_hits = 0;
        self.opened_at = Instant::now();
        Batch::Files(std::mem::take(&mut self.batch_files))
    }

    fn is_empty(&self) -> bool {
        self.batch_files.is_empty()
    }
}

/// Compute byte-offset highlight ranges for the matched line by re-running the matcher.
fn compute_match_ranges(matcher: &grep_regex::RegexMatcher, line_text: &str) -> Vec<(u32, u32)> {
    let mut ranges = Vec::new();
    let mut last_end = 0usize;
    let _ = matcher.find_iter(line_text.as_bytes(), |m| {
        let start = m.start() as u32;
        let end = m.end() as u32;
        if (start as usize) >= last_end {
            ranges.push((start, end));
            last_end = m.end();
        }
        true
    });
    ranges
}
