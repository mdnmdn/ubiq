//! The search worker: walk the project's files, match a pattern, stream batches back.
//!
//! The walk uses `ignore::WalkBuilder` so the project's own `.gitignore` rules are respected.
//! Matching goes through `grep-regex` and `grep-searcher`, which is what ripgrep uses. The worker
//! is interruptible between files via an `Arc<AtomicBool>`.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use grep_regex::RegexMatcherBuilder;
use ubiq_proto::messages::Message;
use ubiq_proto::search::{self, Batch, Source};

use super::Job;
use super::ceiling;
use super::fallback;
use super::hits::{self, State};
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

    // The index, where there is one that can answer this query, names the files worth reading and
    // the walk is skipped. Everything after this point is the same code either way: the same sink,
    // the same batching, the same ceilings, the same messages — the index chose the files and
    // nothing else (`D75`).
    //
    // Note where this sits: **after** the matcher was built above, so a pattern the engine rejects
    // has already gone to the external fallback. A query that uses `ag` takes the same path it
    // always did, index or no index.
    if let Some(candidates) = candidates(job, &start) {
        run_candidates(job, &matcher, candidates, started);
        return;
    }

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

                let hit = hits::scan_file(&matcher, abs_path, rel_path);

                // Every visited file passes through this shared tail, hits or not: `files_seen`,
                // progress, batching and the ceiling checks all depend on seeing every file, not
                // just the ones that matched.
                let mut state = state.lock().unwrap();
                state.saw_file();

                if let Some(hit) = hit {
                    state.add_file(hit);
                }

                let batch = state.should_flush().then(|| state.take_batch());
                let files_seen = state.files_seen;
                let report = files_seen - state.reported_at >= ceiling::PROGRESS_INTERVAL;
                if report {
                    state.reported_at = files_seen;
                }
                let at_ceiling = state.at_ceiling();
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
                if at_ceiling {
                    cancel.store(true, Ordering::Relaxed);
                    return ignore::WalkState::Quit;
                }

                ignore::WalkState::Continue
            })
        }
    });

    // Flush any remaining hits.
    let mut state = state.lock().unwrap();
    finish(job, &mut state, started, "walk");
}

/// The closing messages every search sends, whichever path chose its files.
///
/// Factored so the index path cannot drift from the walk's: the final batch, the true `files_seen`
/// and the `SearchFinished` that carries `truncated` are the contract, and two copies of it would
/// be two chances to send a different one. `how` names the path for the log line and appears
/// nowhere on the wire — the interface is not told which ran, because it must not care (`D75`).
fn finish(job: &Job, state: &mut State, started: Instant, how: &str) {
    let project_id = job.project_id;
    let search_id = job.search_id;
    let truncated = state.at_ceiling();
    let files_seen = state.files_seen;
    let files_with_hits = state.files_with_hits;
    let total_hits = state.total_hits;

    if !state.is_empty() {
        let batch = state.take_batch();
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
        searched: vec![Source::File],
        truncated,
    });

    tracing::info!(
        search = %search_id,
        project = %project_id,
        how,
        files_seen,
        files_with_hits,
        total_hits,
        elapsed_ms = started.elapsed().as_millis(),
        truncated,
        // Set by a cancel, by a supersede, or by a ceiling the search hit — `truncated` tells the
        // last of those apart from the first two.
        stopped_early = job.cancel.load(Ordering::Relaxed),
        "search finished"
    );

    // The flag now also means "this search is over", cancelled or not — the coordinator reaps
    // `active_searches` entries by reading it, not just a cancel request.
    job.cancel.store(true, Ordering::Relaxed);
}

/// The files an index says are worth reading, or `None` when the index cannot answer.
///
/// `None` covers every reason the shortcut does not apply, and they are all the same answer to the
/// caller — walk it:
///
/// - the project keeps no index, or its build has not finished (`ready` is false);
/// - the query is a regular expression, which a term index cannot bound;
/// - the query is shorter than one trigram;
/// - the index errored, which is logged once and then forgotten.
///
/// A subdirectory filter or an include glob does not disqualify the index: the candidates are
/// filtered by both below, which is cheaper than walking the tree to apply the same test.
#[cfg(feature = "index")]
fn candidates(job: &Job, start: &Path) -> Option<Vec<PathBuf>> {
    let index = job.index.as_ref()?;
    if !index.ready() || job.query.regex {
        return None;
    }

    let (paths, truncated) = match index.candidates(&job.query.text) {
        Ok(Some(found)) => found,
        // The query has no trigram, so the index has nothing to say about it.
        Ok(None) => return None,
        Err(error) => {
            tracing::warn!(%error, "the index could not answer; walking instead");
            return None;
        }
    };
    if truncated {
        tracing::debug!(
            candidates = paths.len(),
            "the candidate ceiling was hit; the search is bounded by it"
        );
    }

    // The index knows nothing about this search's filter, so apply it here. `start` is the root
    // unless a subdirectory was asked for, and the overrides carry the include globs and excludes.
    let over = {
        let mut builder = ignore::overrides::OverrideBuilder::new(&job.root);
        for pattern in &job.filter.patterns {
            builder.add(pattern).ok()?;
        }
        for exclude in &job.excludes {
            builder.add(&format!("!{exclude}")).ok()?;
        }
        builder.build().ok()?
    };

    Some(
        paths
            .into_iter()
            .map(|rel| job.root.join(rel))
            .filter(|abs| abs.starts_with(start))
            .filter(|abs| allowed(&over, &job.root, abs))
            .collect(),
    )
}

/// Without `index` there is no shortcut to take: the caller always walks, exactly as it did
/// before any index existed.
#[cfg(not(feature = "index"))]
fn candidates(_job: &Job, _start: &Path) -> Option<Vec<PathBuf>> {
    None
}

/// Whether the filter lets this file through, testing the directories above it as well as itself.
///
/// The walk gets this for free: an exclude naming a directory makes `ignore` prune the whole
/// subtree, so no file inside it is ever visited. There is no traversal here to prune, and a glob
/// like `vendor` matches the *directory* and not `vendor/skip.txt`, so the ancestors have to be
/// asked about one at a time. Missing this is how an excluded folder's files come back the moment
/// a project is indexed, and only then.
#[cfg(feature = "index")]
fn allowed(over: &ignore::overrides::Override, root: &Path, abs: &Path) -> bool {
    // The file itself. This is also where include globs are decided: `Override` answers "ignore"
    // for a file matching no whitelist glob when any whitelist glob exists.
    if over.matched(abs, false).is_ignore() {
        return false;
    }
    let Ok(rel) = abs.strip_prefix(root) else {
        return true;
    };
    let parts: Vec<_> = rel.components().collect();
    let mut dir = root.to_path_buf();
    // Every component but the last, which is the file name tested above.
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        dir.push(part);
        if over.matched(&dir, true).is_ignore() {
            return false;
        }
    }
    true
}

/// Read the candidate files and stream their hits, then finish exactly as the walk does.
///
/// Single-threaded on purpose: the walk parallelises because it is bound by traversing a tree, and
/// this has no tree to traverse — it is a short list of files to read. The `State` and the sink are
/// the walk's own, so the batches are indistinguishable.
fn run_candidates(
    job: &Job,
    matcher: &grep_regex::RegexMatcher,
    candidates: Vec<PathBuf>,
    started: Instant,
) {
    let mut state = State::new();

    for abs_path in &candidates {
        if job.cancel.load(Ordering::Relaxed) {
            break;
        }

        let rel_path = abs_path
            .strip_prefix(&job.root)
            .unwrap_or(abs_path)
            .to_string_lossy()
            .into_owned();

        let hit = hits::scan_file(matcher, abs_path, rel_path);
        state.saw_file();
        if let Some(hit) = hit {
            state.add_file(hit);
        }

        if state.should_flush() {
            let batch = state.take_batch();
            job.reply_to.send(Message::SearchMatches {
                project_id: job.project_id,
                search_id: job.search_id,
                batch,
            });
        }
        if state.files_seen - state.reported_at >= ceiling::PROGRESS_INTERVAL {
            state.reported_at = state.files_seen;
            job.reply_to.send(Message::SearchProgress {
                project_id: job.project_id,
                search_id: job.search_id,
                files_seen: state.files_seen,
            });
        }
        if state.at_ceiling() {
            break;
        }
    }

    finish(job, &mut state, started, "index");
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
