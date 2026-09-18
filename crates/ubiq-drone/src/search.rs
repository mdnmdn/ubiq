//! Content search, by shelling to whatever the machine's own `PATH` already carries.
//!
//! **A drone never builds an index.** It is a guest on somebody else's machine, and the resident
//! cost an index implies — a directory of state that outlives the search that asked for it — is
//! the thing this whole design exists to avoid. So there is no walker here either: a real search
//! tool already knows how to walk a tree faster than anything this crate would write, and porting
//! `ubiq_host::search`'s own walk-and-match engine would be exactly that resident cost in a
//! different shape. What is here instead is a one-shot exec, every time.
//!
//! [`probe`] picks the tool once, in order: `rg` first — its `--json` mode is the only one of the
//! three that hands back exact match ranges rather than a whole highlighted line, and it is the
//! fastest — then `ag`, then a bare `grep -rn`. A machine with none of them answers every search
//! with [`ubiq_proto::search::SearchError::Walk`] naming what was tried, because a refusal that
//! says what to install beats a silent empty result.
//!
//! [`run`] is the worker: it translates a [`Query`] and a [`Filter`] into that tool's own flags —
//! never into a shell string, so a query holding a quote or a semicolon is a query and not an
//! injection — streams its output, and answers with exactly the messages
//! `ubiq_host::search::worker` does, batched the same way and under the same ceilings
//! ([`ubiq_host::search::ceiling`], [`ubiq_host::search::hits::State`]), so the interface needs no
//! branch for a drone's answer.
//!
//! [`Scope::Files`] is the only scope served. Tasks, chats and the knowledge base are the
//! coordinator's own state — a drone holds none of it — so anything else answers
//! [`Message::SearchFinished`] with an empty `searched` rather than an error: nothing here failed,
//! there was simply nothing here that could have answered.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Instant;

use ubiq_host::files;
use ubiq_host::search::ceiling;
use ubiq_host::search::hits::State;
use ubiq_proto::bus::Mailbox;
use ubiq_proto::ids::{ProjectId, SearchId};
use ubiq_proto::messages::Message;
use ubiq_proto::search::{FileHit, Filter, LineHit, Query, SearchError, Source};

/// A tool this drone can shell to, in the order [`detect`] tries them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Rg,
    Ag,
    Grep,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Tool::Rg => "rg",
            Tool::Ag => "ag",
            Tool::Grep => "grep",
        }
    }
}

/// One tool this machine actually has, and where.
pub struct Found {
    tool: Tool,
    program: PathBuf,
}

/// Where a bare program name sits on this process's own `PATH` — the literal environment
/// variable, not the widened one `ubiq_host::shells::repair_path` writes back later. What a drone
/// offers is what the account that started it can actually reach on its own; the extra homes
/// `repair_path` adds are for launching a shell the user picked by name, which is a different
/// question from whether this machine has a search tool at all.
fn locate(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(windows)]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Try each tool in order and say which one, if any, this machine has.
fn detect() -> Option<Found> {
    for tool in [Tool::Rg, Tool::Ag, Tool::Grep] {
        if let Some(program) = locate(tool.name()) {
            tracing::info!(
                tool = tool.name(),
                program = %program.display(),
                "search tool found"
            );
            return Some(Found { tool, program });
        }
    }
    tracing::warn!("no search tool on PATH: tried rg, ag, grep");
    None
}

static FOUND: OnceLock<Option<Found>> = OnceLock::new();

/// The tool this drone shells to for every search from here on, probed once and remembered.
///
/// Called from [`crate::capabilities`] at process start — deliberately ahead of
/// `ubiq_host::shells::repair_path`, which widens this same process's `PATH` a moment later for
/// the sake of spawning a shell by name — so what a drone advertises is what its own account can
/// find, not what a login shell would add on its behalf. The worker below calls it again on a
/// search's first run; both see the same answer, because this is the only place that computes it.
pub fn probe() -> Option<&'static Found> {
    FOUND.get_or_init(detect).as_ref()
}

/// A running search's kill switch, shared between the worker thread that owns the child process
/// and whichever thread answers `CancelSearch` — [`crate::relay::Relay`], on the bus's own thread.
#[derive(Clone)]
pub struct Cancel {
    over: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

impl Cancel {
    pub fn new() -> Self {
        Self {
            over: Arc::new(AtomicBool::new(false)),
            child: Arc::new(Mutex::new(None)),
        }
    }

    /// Mark this search over and kill whatever child it is running, if any.
    ///
    /// Idempotent, on purpose: a search that finished on its own, one a ceiling stopped, and one
    /// the user cancelled all end up here, and killing an already-reaped child is a harmless
    /// no-op. This is also how [`crate::relay::Relay`] reaps `active_searches` — the flag means
    /// "this search is over", not just "somebody asked it to stop".
    pub fn cancel(&self) {
        self.over.store(true, Ordering::Relaxed);
        if let Ok(mut child) = self.child.lock()
            && let Some(child) = child.as_mut()
        {
            let _ = child.kill();
        }
    }

    pub fn is_set(&self) -> bool {
        self.over.load(Ordering::Relaxed)
    }

    /// Record the spawned child so a cancel arriving on another thread can reach it. Kills it
    /// immediately if the cancel already landed in the gap between deciding to spawn and this
    /// call — the race a plain "check before spawning" would miss.
    fn hold(&self, child: Child) {
        let mut guard = self.child.lock().unwrap();
        let already = self.over.load(Ordering::Relaxed);
        *guard = Some(child);
        if already && let Some(child) = guard.as_mut() {
            let _ = child.kill();
        }
    }

    /// Take the child back for reaping, once the stream it wrote to has ended.
    fn take(&self) -> Option<Child> {
        self.child.lock().unwrap().take()
    }
}

impl Default for Cancel {
    fn default() -> Self {
        Self::new()
    }
}

/// One search request, addressed — a drone's mirror of `ubiq_host::search::Job`, minus the index
/// (a drone keeps none) and the merged excludes (a drone holds no `Settings` to merge from;
/// `filter.patterns` is all it has to go on).
pub struct Job {
    pub project_id: ProjectId,
    pub search_id: SearchId,
    pub root: PathBuf,
    pub query: Query,
    pub filter: Filter,
    pub cancel: Cancel,
    pub reply_to: Mailbox,
}

/// The thread that answers the search family by shelling out.
///
/// One thread, not a pool, for the same reason `ubiq_host::search::Search` is: a second search for
/// a project supersedes the first rather than queuing behind it, so ordering within one project is
/// never in question.
pub struct Search {
    jobs: flume::Sender<Job>,
}

impl Search {
    pub fn start() -> Self {
        let (jobs, queue) = flume::unbounded::<Job>();
        thread::Builder::new()
            .name("ubiq-drone-search".to_string())
            .spawn(move || {
                while let Ok(job) = queue.recv() {
                    run(job);
                }
            })
            .expect("the drone search thread");
        Self { jobs }
    }

    /// Queue a search. Never blocks — the queue is unbounded, on the bus's own rule.
    pub fn submit(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            tracing::error!("the drone search thread has gone; a request was dropped");
        }
    }
}

/// Run one search job: spawn the tool, stream its output, answer as the local host would.
fn run(job: Job) {
    let project_id = job.project_id;
    let search_id = job.search_id;
    let started = Instant::now();

    tracing::info!(
        search = %search_id,
        project = %project_id,
        root = %job.root.display(),
        query_len = job.query.text.len(),
        regex = job.query.regex,
        patterns = job.filter.patterns.len(),
        narrowed = job.filter.subdir.is_some(),
        "drone search started"
    );

    let Some(found) = probe() else {
        job.reply_to.send(Message::SearchError {
            project_id,
            search_id,
            error: SearchError::Walk(
                "no search tool on this machine's PATH: install ripgrep (rg), the silver \
                 searcher (ag), or grep"
                    .to_string(),
            ),
        });
        job.cancel.cancel();
        return;
    };

    let start = match &job.filter.subdir {
        Some(subdir) => match files::path::resolve(&job.root, subdir) {
            Ok(path) if path.is_dir() => path,
            Ok(_) => return finish_bad_filter(&job, "that path is not a directory".to_string()),
            Err(error) => return finish_bad_filter(&job, error.to_string()),
        },
        None => job.root.clone(),
    };

    let overrides = match build_overrides(&job.root, &job.filter.patterns) {
        Ok(overrides) => overrides,
        Err(error) => return finish_bad_filter(&job, error),
    };

    // A search cancelled or superseded before its tool ever ran still gets an answer — every
    // message gets one, the rule this whole relay is built on. It is a quick one: the child below
    // is spawned and `Cancel::hold` kills it in the same breath, so the loop after it reads
    // nothing before `finish` sends an empty `SearchFinished`.
    let mut command = build_command(found, &job.query, &start);
    let mut child = match command.stdout(Stdio::piped()).stderr(Stdio::null()).spawn() {
        Ok(child) => child,
        Err(error) => {
            job.reply_to.send(Message::SearchError {
                project_id,
                search_id,
                error: SearchError::Walk(format!("{} did not start: {error}", found.tool.name())),
            });
            job.cancel.cancel();
            return;
        }
    };
    let stdout = child.stdout.take().expect("stdout was piped");
    job.cancel.hold(child);

    let parse: fn(&str) -> Option<RawHit> = match found.tool {
        Tool::Rg => parse_rg_line,
        Tool::Ag | Tool::Grep => parse_text_line,
    };

    let mut state = State::new();
    let mut current: Option<(String, Vec<LineHit>)> = None;

    for line in BufReader::new(stdout).lines() {
        if job.cancel.is_set() {
            break;
        }
        let Ok(line) = line else { continue };
        let Some(raw) = parse(&line) else { continue };
        let rel_path = rel_to_root(&raw.rel_path, &job.root);
        if let Some(overrides) = &overrides
            && overrides
                .matched(job.root.join(&rel_path), false)
                .is_ignore()
        {
            continue;
        }
        let hit = LineHit {
            line: raw.line,
            text: raw.text,
            ranges: raw.ranges,
        };

        match &mut current {
            Some((current_path, lines)) if *current_path == rel_path => {
                if lines.len() < ceiling::HITS_PER_FILE {
                    lines.push(hit);
                }
            }
            _ => {
                flush_current(&mut state, &mut current);
                current = Some((rel_path, vec![hit]));
                state.saw_file();
            }
        }

        if let Some(batch) = state.should_flush().then(|| state.take_batch()) {
            job.reply_to.send(Message::SearchMatches {
                project_id,
                search_id,
                batch,
            });
        }
        if state.files_seen.saturating_sub(state.reported_at) >= ceiling::PROGRESS_INTERVAL {
            state.reported_at = state.files_seen;
            job.reply_to.send(Message::SearchProgress {
                project_id,
                search_id,
                files_seen: state.files_seen,
            });
        }
        if state.at_ceiling() {
            job.cancel.cancel();
            break;
        }
    }
    flush_current(&mut state, &mut current);

    if let Some(mut child) = job.cancel.take() {
        let _ = child.wait();
    }

    let truncated = state.at_ceiling();
    if !state.is_empty() {
        let batch = state.take_batch();
        job.reply_to.send(Message::SearchMatches {
            project_id,
            search_id,
            batch,
        });
    }
    job.reply_to.send(Message::SearchProgress {
        project_id,
        search_id,
        files_seen: state.files_seen,
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
        tool = found.tool.name(),
        files_seen = state.files_seen,
        files_with_hits = state.files_with_hits,
        total_hits = state.total_hits,
        elapsed_ms = started.elapsed().as_millis(),
        truncated,
        "drone search finished"
    );

    // The flag now also means "this search is over", the same double duty
    // `ubiq_host::search::worker::finish` gives it — `Relay` reaps `active_searches` by reading it.
    job.cancel.cancel();
}

fn finish_bad_filter(job: &Job, error: String) {
    job.reply_to.send(Message::SearchError {
        project_id: job.project_id,
        search_id: job.search_id,
        error: SearchError::BadFilter(error),
    });
    job.cancel.cancel();
}

/// The file just finished, if any, folded into `state` as one [`FileHit`].
fn flush_current(state: &mut State, current: &mut Option<(String, Vec<LineHit>)>) {
    if let Some((rel_path, lines)) = current.take() {
        let truncated = lines.len() >= ceiling::HITS_PER_FILE;
        state.add_file(FileHit {
            rel_path,
            lines,
            truncated,
        });
    }
}

/// `filter.patterns` as gitignore-style globs against the project-relative path — the same
/// convention `ubiq_proto::search::Filter` documents and `ubiq_host::search::worker` applies to
/// its own index shortcut. `None` is "everything passes", which is what an empty list means.
fn build_overrides(
    root: &Path,
    patterns: &[String],
) -> Result<Option<ignore::overrides::Override>, String> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = ignore::overrides::OverrideBuilder::new(root);
    for pattern in patterns {
        builder.add(pattern).map_err(|error| error.to_string())?;
    }
    builder.build().map(Some).map_err(|error| error.to_string())
}

/// Translate a query into the chosen tool's own flags. Every piece of user text is its own
/// argument — `Command::arg`, never a shell string — so a quote or a semicolon in a query is a
/// query and not an injection.
fn build_command(found: &Found, query: &Query, start: &Path) -> Command {
    let mut command = Command::new(&found.program);
    // A drone's own search tool runs headlessly, piped straight into this process — there is no
    // console output for a user to read. On Windows a console-subsystem child (`rg.exe`, `grep.exe`)
    // spawned with none of its own would otherwise flash a fresh console window, so this asks for
    // none.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    match found.tool {
        Tool::Rg => {
            // `--json` is the reason `rg` is tried first: submatch byte ranges, not a whole line
            // guessed at. `--sort path` trades rg's default per-file parallelism for a
            // deterministic, single-threaded order, which is what lets the streaming parser below
            // group a file's hits by simple adjacency instead of a `HashMap`.
            command.arg("--json").arg("--sort").arg("path");
            command.arg(if query.case_sensitive { "-s" } else { "-i" });
            if query.whole_word {
                command.arg("-w");
            }
            if !query.regex {
                command.arg("-F");
            }
            command.arg("-e").arg(&query.text).arg("--").arg(start);
        }
        Tool::Ag => {
            // `--workers 1` is `rg`'s `--sort path` for a tool with no sort flag of its own: it
            // is what keeps one file's lines contiguous in the output.
            command
                .arg("--numbers")
                .arg("--nocolor")
                .arg("--nogroup")
                .arg("--workers")
                .arg("1");
            if !query.case_sensitive {
                command.arg("-i");
            }
            if query.whole_word {
                command.arg("-w");
            }
            if !query.regex {
                command.arg("--literal");
            }
            command.arg("--").arg(&query.text).arg(start);
        }
        Tool::Grep => {
            command.arg("-rn").arg("--binary-files=without-match");
            if !query.case_sensitive {
                command.arg("-i");
            }
            if query.whole_word {
                command.arg("-w");
            }
            command.arg(if query.regex { "-E" } else { "-F" });
            command.arg("-e").arg(&query.text).arg("--").arg(start);
        }
    }
    command
}

/// One line of a tool's output, before it becomes a [`LineHit`].
struct RawHit {
    rel_path: String,
    line: u32,
    text: String,
    ranges: Vec<(u32, u32)>,
}

/// `rg --json`'s `match` records; every other record type (`begin`, `end`, `summary`) is `None`,
/// the same as a line the text parser below does not recognise.
fn parse_rg_line(line: &str) -> Option<RawHit> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value.get("type")?.as_str()? != "match" {
        return None;
    }
    let data = value.get("data")?;
    let rel_path = data.get("path")?.get("text")?.as_str()?.to_string();
    let line_number = data.get("line_number")?.as_u64()? as u32;
    let mut text = data.get("lines")?.get("text")?.as_str()?.to_string();
    while text.ends_with(['\n', '\r']) {
        text.pop();
    }
    let ranges = data
        .get("submatches")?
        .as_array()?
        .iter()
        .filter_map(|sub| {
            let start = sub.get("start")?.as_u64()? as u32;
            let end = sub.get("end")?.as_u64()? as u32;
            Some((start, end))
        })
        .collect();
    Some(RawHit {
        rel_path,
        line: line_number,
        text,
        ranges,
    })
}

/// `ag`'s and `grep`'s shared `path:line:text` output.
///
/// Split on every colon, not just the first two: a path may itself hold a colon, so the split
/// walks from the left for the first purely-numeric segment — that is the line number, whatever
/// index it falls at — and rejoins what is before it as the path and what is after it as the text,
/// colons and all. Neither tool tells us a match's byte range, so the whole line is the highlight,
/// exactly as `ubiq_host::search::fallback` already does for the same reason.
fn parse_text_line(line: &str) -> Option<RawHit> {
    let segments: Vec<&str> = line.split(':').collect();
    let (at, line_number) = segments
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(at, segment)| segment.parse::<u32>().ok().map(|number| (at, number)))?;
    // Index 0 alone, with no numeric segment after it, is not `path:line:text` at all — a
    // `grep --binary-files=without-match` skip line, or anything else with no colon in it.
    if at == 0 {
        return None;
    }
    let rel_path = segments[..at].join(":");
    let text = segments[at + 1..].join(":");
    Some(RawHit {
        rel_path,
        line: line_number,
        ranges: vec![(0, text.len() as u32)],
        text,
    })
}

/// Project-relative, the same convention `ubiq_host::search` uses.
fn rel_to_root(path: &str, root: &Path) -> String {
    Path::new(path)
        .strip_prefix(root)
        .unwrap_or_else(|_| Path::new(path))
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_path_holding_a_colon() {
        let hit = parse_text_line("weird:name.rs:5:hello world").expect("a hit");
        assert_eq!(hit.rel_path, "weird:name.rs");
        assert_eq!(hit.line, 5);
        assert_eq!(hit.text, "hello world");
    }

    #[test]
    fn parses_a_line_whose_text_holds_a_colon() {
        let hit = parse_text_line("src/main.rs:12:let x: u32 = 1;").expect("a hit");
        assert_eq!(hit.rel_path, "src/main.rs");
        assert_eq!(hit.line, 12);
        assert_eq!(hit.text, "let x: u32 = 1;");
    }

    #[test]
    fn a_binary_file_notice_is_not_a_hit() {
        assert!(parse_text_line("Binary file src/data.bin matches").is_none());
        assert!(parse_text_line("not a hit line either").is_none());
    }

    #[test]
    fn parses_an_rg_json_match_with_its_submatch_ranges() {
        let line = r#"{"type":"match","data":{"path":{"text":"src/lib.rs"},"lines":{"text":"fn needle() {}\n"},"line_number":3,"submatches":[{"match":{"text":"needle"},"start":3,"end":9}]}}"#;
        let hit = parse_rg_line(line).expect("a hit");
        assert_eq!(hit.rel_path, "src/lib.rs");
        assert_eq!(hit.line, 3);
        assert_eq!(hit.text, "fn needle() {}");
        assert_eq!(hit.ranges, vec![(3, 9)]);
    }

    #[test]
    fn an_rg_json_summary_line_is_not_a_hit() {
        let line = r#"{"type":"summary","data":{"elapsed_total":{"secs":0}}}"#;
        assert!(parse_rg_line(line).is_none());
    }

    #[test]
    fn detect_never_picks_find_or_fd() {
        // Neither is named in `Tool`, so a machine that has only them still probes empty — the
        // same rule `ubiq_host::search::fallback::pick` states for the same two names: a file-name
        // matcher cannot answer a content search, and running it as one would misread its output.
        // Nothing to assert against the real machine here; this documents the omission is by
        // construction rather than by the environment this test happens to run in.
        for tool in [Tool::Rg, Tool::Ag, Tool::Grep] {
            assert_ne!(tool.name(), "find");
            assert_ne!(tool.name(), "fd");
        }
    }
}
