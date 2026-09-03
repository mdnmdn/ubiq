//! The search worker: walk the project's files, match a pattern, stream batches back.
//!
//! The walk uses `ignore::WalkBuilder` so the project's own `.gitignore` rules are respected.
//! Matching goes through `grep-regex` and `grep-searcher`, which is what ripgrep uses. The worker
//! is interruptible between files via an `Arc<AtomicBool>`.

use std::cmp;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use grep_matcher::Matcher;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use ubiq_proto::messages::Message;
use ubiq_proto::search::{self, Batch, FileHit, LineHit, Source};

use super::Job;
use super::ceiling;

/// Run one search job. Sends [`Message::SearchMatches`] batches, periodic
/// [`Message::SearchProgress`], and a final [`Message::SearchFinished`].
pub fn run(job: &Job) {
    let project_id = job.project_id;
    let search_id = job.search_id;

    // Build the matcher from the query. A bad pattern is a [`SearchError::BadQuery`], not a panic.
    let matcher = match RegexMatcherBuilder::new()
        .case_insensitive(!job.query.case_sensitive)
        .fixed_strings(!job.query.regex)
        .word(job.query.whole_word)
        .build(&job.query.text)
    {
        Ok(matcher) => Arc::new(matcher),
        Err(error) => {
            job.reply_to.send(Message::SearchError {
                project_id,
                search_id,
                error: search::SearchError::BadQuery(error.to_string()),
            });
            return;
        }
    };

    // Walk the project tree. `ignore` respects `.gitignore`, `.ignore`, and hidden files.
    let walker = ignore::WalkBuilder::new(&job.root)
        .threads(cmp::min(num_cpus::get(), 8))
        .build_parallel();

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

                if file_hits.is_empty() {
                    return ignore::WalkState::Continue;
                }

                let truncated = file_hits.len() >= ceiling::HITS_PER_FILE;
                let hit = FileHit {
                    rel_path,
                    lines: file_hits,
                    truncated,
                };

                // Accumulate and possibly flush a batch.
                let mut state = state.lock().unwrap();
                state.add_file(hit);

                if state.should_flush() {
                    let batch = state.take_batch();
                    let files_seen = state.files_seen;
                    let total_hits = state.total_hits;
                    // Drop the lock before sending.
                    drop(state);

                    reply_to.send(Message::SearchMatches {
                        project_id,
                        search_id,
                        batch,
                    });

                    // Progress report at intervals.
                    if files_seen.is_multiple_of(ceiling::PROGRESS_INTERVAL) {
                        reply_to.send(Message::SearchProgress {
                            project_id,
                            search_id,
                            files_seen,
                        });
                    }

                    // Check ceilings.
                    if total_hits >= ceiling::TOTAL_HITS {
                        cancel.store(true, Ordering::Relaxed);
                        return ignore::WalkState::Quit;
                    }
                }

                ignore::WalkState::Continue
            })
        }
    });

    // Flush any remaining hits.
    let mut state = state.lock().unwrap();
    let truncated = state.total_hits >= ceiling::TOTAL_HITS
        || state.files_with_hits() >= ceiling::FILES_WITH_HITS;
    let searched = vec![Source::File];

    if !state.is_empty() {
        let batch = state.take_batch();
        job.reply_to.send(Message::SearchMatches {
            project_id,
            search_id,
            batch,
        });
    }

    job.reply_to.send(Message::SearchFinished {
        project_id,
        search_id,
        searched,
        truncated,
    });
}

/// Worker-level state, protected by a mutex across the parallel walk's file-level serialisation.
struct State {
    batch_files: Vec<FileHit>,
    files_seen: usize,
    total_hits: usize,
    /// Accumulated hits since last flush.
    pending_hits: usize,
}

impl State {
    fn new() -> Self {
        Self {
            batch_files: Vec::new(),
            files_seen: 0,
            total_hits: 0,
            pending_hits: 0,
        }
    }

    fn add_file(&mut self, hit: FileHit) {
        self.files_seen += 1;
        self.pending_hits += hit.lines.len();
        self.total_hits += hit.lines.len();
        self.batch_files.push(hit);
    }

    fn should_flush(&self) -> bool {
        self.batch_files.len() >= 64 || self.pending_hits >= 512
    }

    fn take_batch(&mut self) -> Batch {
        self.pending_hits = 0;
        Batch::Files(std::mem::take(&mut self.batch_files))
    }

    fn is_empty(&self) -> bool {
        self.batch_files.is_empty()
    }

    fn files_with_hits(&self) -> usize {
        self.batch_files.len()
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
