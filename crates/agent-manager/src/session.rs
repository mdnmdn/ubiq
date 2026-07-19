//! Session history: metadata + transcript for each `am`-launched run.
//!
//! Every real run (passthrough or structured) is recorded under `am`'s own
//! state dir as `<sessions-root>/<id>/{meta.json,transcript.jsonl}` — never
//! under the user's real harness config (`~/.claude`, `~/.codex`,
//! `~/.config/opencode`, ...). This module owns the on-disk shape and the
//! read/write access to it; `am session ls|show` (see `src/cli/session.rs`)
//! is a thin presentation layer on top, and `am <harness>` (see
//! `src/cli/run.rs`) is the only writer.
//!
//! Resume (`am session resume`) is a later step (F2); the shapes here
//! (particularly [`SessionMeta::config_dir`] and
//! [`SessionMeta::harness_session_id`]) are already sized for it, but this
//! module does not implement resume itself.
//!
//! Mirrors [`crate::account`]'s `AM_ACCOUNTS`-override pattern (see
//! [`crate::account::resolve_accounts_root`]) with an `AM_SESSIONS`
//! equivalent, and reuses [`crate::provision`]'s `<unix-millis>-<pid>` id
//! scheme (see `new_run_dir` there) for session ids.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::io::AgentEvent;
use crate::Result;

/// Recorded metadata for one `am`-launched run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    /// Session id (`<unix-millis>-<pid>`, same scheme as ephemeral run dirs).
    pub id: String,
    /// The harness id that was run (e.g. `"claude-code"`).
    pub harness: String,
    /// Working directory the harness ran in.
    pub cwd: PathBuf,
    /// The launch program + args actually run (what `--print-config` would
    /// have shown).
    pub argv: Vec<String>,
    /// The account id used, if any.
    pub account: Option<String>,
    /// `"passthrough"` or `"structured"`.
    pub io: String,
    /// The provisioned config dir for this run. May no longer exist by the
    /// time this is read (ephemeral dirs are cleaned up after the run) —
    /// kept for a future resume step.
    pub config_dir: PathBuf,
    /// Unix millis when the session started.
    pub created_at: u64,
    /// Unix millis when the session finished, if it has.
    pub finished_at: Option<u64>,
    /// The child's exit code, if the run finished.
    pub exit_code: Option<i32>,
    /// The harness's own session id, if one was captured from an
    /// [`AgentEvent::SessionStarted`] event (structured runs only, this
    /// step).
    pub harness_session_id: Option<String>,
}

impl SessionMeta {
    /// Build a fresh, in-progress `SessionMeta` for a run that's about to
    /// start: assigns a new id (see [`new_session_id`]) and stamps
    /// `created_at` now; `finished_at`/`exit_code`/`harness_session_id` all
    /// start unset.
    pub fn new(
        harness: String,
        cwd: PathBuf,
        argv: Vec<String>,
        account: Option<String>,
        io: String,
        config_dir: PathBuf,
    ) -> Self {
        SessionMeta {
            id: new_session_id(),
            harness,
            cwd,
            argv,
            account,
            io,
            config_dir,
            created_at: now_millis(),
            finished_at: None,
            exit_code: None,
            harness_session_id: None,
        }
    }
}

/// The default sessions root: `~/.config/agent-manager/sessions` on all
/// platforms — the same base dir as the config file
/// ([`crate::settings::default_config_dir`]), so every agent-manager store
/// lives together under `~/.config/agent-manager/`. Overridable by
/// `AM_SESSIONS` (see [`sessions_root`]).
fn default_sessions_root() -> Option<PathBuf> {
    crate::settings::default_config_dir().map(|d| d.join("sessions"))
}

/// Resolve the sessions root from (highest first): an explicit path, the
/// `AM_SESSIONS` env var, then the default. Returns `None` if none apply.
pub fn sessions_root(explicit: Option<PathBuf>) -> Option<PathBuf> {
    explicit
        .or_else(|| std::env::var("AM_SESSIONS").ok().map(PathBuf::from))
        .or_else(default_sessions_root)
}

/// Generate a fresh session id: `<unix-millis>-<pid>` (same scheme as
/// [`crate::provision::new_run_dir`]'s run ids).
pub fn new_session_id() -> String {
    format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        std::process::id()
    )
}

/// Current unix-millis timestamp, used for `created_at`/`finished_at`.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

const META_FILE: &str = "meta.json";
const TRANSCRIPT_FILE: &str = "transcript.jsonl";

/// A live recorder sink for a session's transcript. Trait so an embedder can
/// record a run to a database; the CLI uses [`FsSessionRecorder`]. `finish`
/// takes `Box<Self>` so it can be called on a boxed trait object.
pub trait SessionRecorder {
    /// The session id being recorded.
    fn id(&self) -> &str;
    /// Append `event` to the transcript.
    fn record_event(&mut self, event: &AgentEvent) -> Result<()>;
    /// Finalize the session with its exit code.
    fn finish(self: Box<Self>, exit_code: Option<i32>) -> Result<()>;
}

/// A store of recorded sessions — the read side (`list`/`load`/
/// `read_transcript`) plus `start` for a new recording. Trait so an embedder
/// can persist session history in a database; the CLI uses [`FsSessionStore`].
/// See `_docs/am-as-library.md`.
pub trait SessionStore {
    /// Begin recording a new session, returning its live [`SessionRecorder`].
    fn start(&self, meta: SessionMeta) -> Result<Box<dyn SessionRecorder>>;
    /// All recorded sessions, newest first.
    fn list(&self) -> Result<Vec<SessionMeta>>;
    /// One session's metadata by id.
    fn load(&self, id: &str) -> Result<SessionMeta>;
    /// One session's transcript events, in order.
    fn read_transcript(&self, id: &str) -> Result<Vec<AgentEvent>>;
}

/// Filesystem-backed [`SessionStore`] rooted at a sessions directory.
#[derive(Debug, Clone)]
pub struct FsSessionStore {
    root: PathBuf,
}

impl FsSessionStore {
    /// Create a session store rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        FsSessionStore { root: root.into() }
    }
}

impl SessionStore for FsSessionStore {
    fn start(&self, meta: SessionMeta) -> Result<Box<dyn SessionRecorder>> {
        Ok(Box::new(start(&self.root, meta)?))
    }
    fn list(&self) -> Result<Vec<SessionMeta>> {
        list(&self.root)
    }
    fn load(&self, id: &str) -> Result<SessionMeta> {
        load(&self.root, id)
    }
    fn read_transcript(&self, id: &str) -> Result<Vec<AgentEvent>> {
        read_transcript(&self.root, id)
    }
}

impl SessionRecorder for FsSessionRecorder {
    fn id(&self) -> &str {
        FsSessionRecorder::id(self)
    }
    fn record_event(&mut self, event: &AgentEvent) -> Result<()> {
        FsSessionRecorder::record_event(self, event)
    }
    fn finish(self: Box<Self>, exit_code: Option<i32>) -> Result<()> {
        FsSessionRecorder::finish(*self, exit_code)
    }
}

/// Writes a session's `meta.json` + appends its `transcript.jsonl` as the run
/// progresses. Created by [`start`]; call [`FsSessionRecorder::finish`] when
/// the run completes so `finished_at`/`exit_code`/`harness_session_id` get
/// folded into `meta.json`.
pub struct FsSessionRecorder {
    dir: PathBuf,
    meta: SessionMeta,
    transcript: std::fs::File,
    captured_harness_session_id: Option<String>,
}

/// Start recording a new session under `root`: creates `<root>/<id>/`,
/// writes the initial `meta.json`, and opens `transcript.jsonl` for append.
pub fn start(root: &Path, meta: SessionMeta) -> Result<FsSessionRecorder> {
    let dir = root.join(&meta.id);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating session dir {}", dir.display()))?;

    write_meta(&dir, &meta)?;

    let transcript = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(TRANSCRIPT_FILE))
        .with_context(|| format!("opening transcript for session {}", meta.id))?;

    Ok(FsSessionRecorder {
        dir,
        meta,
        transcript,
        captured_harness_session_id: None,
    })
}

impl FsSessionRecorder {
    /// The session id being recorded.
    pub fn id(&self) -> &str {
        &self.meta.id
    }

    /// Append `event` as one JSON line to `transcript.jsonl`. If it's a
    /// [`AgentEvent::SessionStarted`] carrying a harness session id, that id
    /// is remembered and folded into `meta.json` on [`Self::finish`].
    pub fn record_event(&mut self, event: &AgentEvent) -> Result<()> {
        if let AgentEvent::SessionStarted {
            session_id: Some(id),
        } = event
        {
            self.captured_harness_session_id = Some(id.clone());
        }

        let line = serde_json::to_string(event).context("serializing event for transcript")?;
        writeln!(self.transcript, "{line}").context("writing transcript line")?;
        Ok(())
    }

    /// Finalize the session: set `finished_at`, `exit_code`, and (if
    /// captured) `harness_session_id`, then rewrite `meta.json`.
    pub fn finish(mut self, exit_code: Option<i32>) -> Result<()> {
        self.meta.finished_at = Some(now_millis());
        self.meta.exit_code = exit_code;
        if self.meta.harness_session_id.is_none() {
            self.meta.harness_session_id = self.captured_harness_session_id.clone();
        }
        write_meta(&self.dir, &self.meta)
    }
}

fn write_meta(dir: &Path, meta: &SessionMeta) -> Result<()> {
    let json = serde_json::to_string_pretty(meta).context("serializing session meta")?;
    std::fs::write(dir.join(META_FILE), json)
        .with_context(|| format!("writing {}", dir.join(META_FILE).display()))
}

/// List all recorded sessions under `root`, newest first (`created_at`
/// descending). Subdirectories without a readable/parseable `meta.json` are
/// silently skipped.
pub fn list(root: &Path) -> Result<Vec<SessionMeta>> {
    let mut sessions = Vec::new();

    if !root.is_dir() {
        return Ok(sessions);
    }

    for entry in std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let meta_path = entry.path().join(META_FILE);
        let Ok(content) = std::fs::read_to_string(&meta_path) else {
            continue;
        };
        let Ok(meta) = serde_json::from_str::<SessionMeta>(&content) else {
            continue;
        };
        sessions.push(meta);
    }

    sessions.sort_by_key(|s| std::cmp::Reverse(s.created_at));
    Ok(sessions)
}

/// Load one session's metadata by id.
pub fn load(root: &Path, id: &str) -> Result<SessionMeta> {
    let meta_path = root.join(id).join(META_FILE);
    let content = std::fs::read_to_string(&meta_path)
        .with_context(|| format!("no session '{id}' found under {}", root.display()))?;
    let meta: SessionMeta = serde_json::from_str(&content)
        .with_context(|| format!("parsing {}", meta_path.display()))?;
    Ok(meta)
}

/// Path to a session's transcript file (may not exist, e.g. passthrough
/// runs which record metadata only).
pub fn transcript_path(root: &Path, id: &str) -> PathBuf {
    root.join(id).join(TRANSCRIPT_FILE)
}

/// Read all events from a session's transcript, in order. Errors if the
/// transcript can't be read; a malformed line is skipped.
pub fn read_transcript(root: &Path, id: &str) -> Result<Vec<AgentEvent>> {
    let path = transcript_path(root, id);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = std::fs::File::open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let mut events = Vec::new();
    for line in reader.lines() {
        let line = line.with_context(|| format!("reading {}", path.display()))?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(ev) = serde_json::from_str::<AgentEvent>(&line) {
            events.push(ev);
        }
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_meta(id: &str) -> SessionMeta {
        SessionMeta {
            id: id.to_string(),
            harness: "claude-code".to_string(),
            cwd: PathBuf::from("/tmp/project"),
            argv: vec!["claude".to_string()],
            account: None,
            io: "structured".to_string(),
            config_dir: PathBuf::from("/tmp/config"),
            created_at: now_millis(),
            finished_at: None,
            exit_code: None,
            harness_session_id: None,
        }
    }

    #[test]
    fn record_events_then_finish_folds_harness_session_id_and_exit_code() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();

        let mut recorder = start(root, sample_meta("111-222")).unwrap();
        assert_eq!(recorder.id(), "111-222");

        recorder
            .record_event(&AgentEvent::SessionStarted {
                session_id: Some("harness-abc".to_string()),
            })
            .unwrap();
        recorder
            .record_event(&AgentEvent::AssistantText {
                text: "hi".to_string(),
            })
            .unwrap();

        recorder.finish(Some(0)).unwrap();

        let sessions = list(root).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "111-222");
        assert_eq!(sessions[0].exit_code, Some(0));
        assert_eq!(
            sessions[0].harness_session_id.as_deref(),
            Some("harness-abc")
        );
        assert!(sessions[0].finished_at.is_some());

        let transcript = std::fs::read_to_string(transcript_path(root, "111-222")).unwrap();
        assert_eq!(transcript.lines().count(), 2);
    }

    #[test]
    fn load_round_trips_meta() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();

        let recorder = start(root, sample_meta("333-444")).unwrap();
        recorder.finish(Some(1)).unwrap();

        let loaded = load(root, "333-444").unwrap();
        assert_eq!(loaded.id, "333-444");
        assert_eq!(loaded.exit_code, Some(1));
    }

    #[test]
    fn load_missing_session_is_an_error() {
        let temp = tempfile::TempDir::new().unwrap();
        let err = load(temp.path(), "nope").unwrap_err();
        assert!(err.to_string().contains("nope"), "message was: {err}");
    }

    #[test]
    fn list_on_empty_or_missing_root_is_empty() {
        let temp = tempfile::TempDir::new().unwrap();
        assert!(list(temp.path()).unwrap().is_empty());
        assert!(list(&temp.path().join("does-not-exist")).unwrap().is_empty());
    }

    #[test]
    fn list_sorts_by_created_at_descending_and_skips_malformed_dirs() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();

        let mut older = sample_meta("a-old");
        older.created_at = 100;
        start(root, older).unwrap().finish(None).unwrap();

        let mut newer = sample_meta("b-new");
        newer.created_at = 200;
        start(root, newer).unwrap().finish(None).unwrap();

        // A stray directory with no meta.json at all should be skipped.
        std::fs::create_dir_all(root.join("junk")).unwrap();

        let sessions = list(root).unwrap();
        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["b-new", "a-old"]);
    }

    #[test]
    fn read_transcript_round_trips_events() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();

        let mut recorder = start(root, sample_meta("555-666")).unwrap();
        recorder
            .record_event(&AgentEvent::AssistantText {
                text: "hello".to_string(),
            })
            .unwrap();
        recorder
            .record_event(&AgentEvent::Result {
                success: true,
                error: None,
            })
            .unwrap();
        recorder.finish(Some(0)).unwrap();

        let events = read_transcript(root, "555-666").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            AgentEvent::AssistantText {
                text: "hello".to_string()
            }
        );
    }

    #[test]
    fn sessions_root_explicit_override_wins() {
        // `AM_SESSIONS`/the default aren't exercised here: mutating process
        // env in a test is unsafe as of this crate's Rust edition (and this
        // crate forbids `unsafe`), so this only proves the explicit-path
        // precedence, which is what `start`/`list`/`load` actually rely on.
        let explicit = PathBuf::from("/tmp/explicit-sessions-root");
        assert_eq!(sessions_root(Some(explicit.clone())), Some(explicit));
    }
}
