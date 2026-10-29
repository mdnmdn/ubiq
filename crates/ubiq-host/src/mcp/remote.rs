//! The `ubiq-remote` server: an agent bound to a drone working on the far machine (`D226`).
//!
//! The host holds no drone link — the window does (`D116`) — so every tool is a round trip to a
//! window, on the browser server's shape: the call files a row in [`RemoteCalls`], says
//! [`Message::RemoteToolCall`] through the host's own voice — the coordinator addresses it to the
//! window that owns the calling agent, or one that has the project open — and waits for that
//! window's relayed [`Message::RemoteToolResult`]: [`TIMEOUT_SECS`] for a file op, the run's own
//! `timeout_ms` plus [`RUN_MARGIN_SECS`] for `remote_run`. Always on a thread [`super::server`]
//! spawned for the call, never the listener's own.
//!
//! **Which remote is the agent's, never the model's.** [`AgentFacts::remote`] is set at launch
//! from [`Message::StartConversation`]'s `remote`; a call from an agent with none is refused. A
//! call's `root` is the model's pick among the roots `remote_info` listed.
//!
//! The drone answers in [`RemoteOutput`]; [`format`] turns that into compact text for a model —
//! a read numbered like `cat -n`, a run as its exit and the tails of its two streams.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;

use serde_json::Value;
use ubiq_proto::bus::{ClientId, Voice};
use ubiq_proto::files::EntryKind;
use ubiq_proto::ids::{ProjectId, RemoteCallId};
use ubiq_proto::messages::Message;
use ubiq_proto::remote_tools::{RemoteInfo, RemoteOp, RemoteOutput, RemoteRun};

use super::RemoteReach;
use super::registry::AgentFacts;

/// The most remote calls served at once; the next is refused as busy.
pub const MAX_CALLS: usize = 8;

/// How long a file op waits for the window's relayed result.
pub const TIMEOUT_SECS: u64 = 30;

/// What `remote_run` waits past its own `timeout_ms`: the drone kills the process at the bound,
/// and the answer still has to cross two links.
pub const RUN_MARGIN_SECS: u64 = 15;

/// `remote_run`'s default and ceiling for `timeout_ms`.
const RUN_DEFAULT_MS: u64 = 120_000;
const RUN_MAX_MS: u64 = 600_000;

/// `remote_grep`'s default and ceiling for `max_results`.
const GREP_DEFAULT: u32 = 200;
const GREP_MAX: u32 = 2_000;

/// What a window that has gone, or none at all, leaves the agent reading.
pub const NO_WINDOW: &str = "no Ubiq window holding this remote's link is open";

/// The directory, under the project, an agent bound to `label` runs in: a scratch folder that is
/// deliberately not the target system.
pub fn workdir(project_root: &Path, label: &str) -> PathBuf {
    project_root
        .join(crate::store::project_dir::IN_PROJECT_DIR)
        .join("remote")
        .join(sanitise(label))
}

/// [`workdir`] for a project with no folder on this machine — one a drone serves: the same
/// per-remote scratch folder, under the project's own data directory
/// ([`crate::store::project_dir::ProjectData::remote_scratch`]) rather than its folder.
pub fn drone_workdir(scratch: &Path, label: &str) -> PathBuf {
    scratch.join(sanitise(label))
}

/// A label as one safe path segment: anything but ASCII alphanumerics, `-`, `_` and `.` becomes
/// `-`, and a segment that would be empty or a dot-name is `remote`.
pub fn sanitise(label: &str) -> String {
    let cleaned: String = label
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(['-', '.']).to_string();
    if cleaned.is_empty() {
        "remote".to_string()
    } else {
        cleaned
    }
}

/// The always-on instructions appended for an agent bound to `label`. Names no harness: the
/// library decides where instructions go.
pub fn instructions(label: &str) -> String {
    format!(
        "You are working on the remote machine \"{label}\", through the ubiq-remote tools \
         (remote_info, remote_list, remote_read, remote_write, remote_edit, remote_glob, \
         remote_grep, remote_run) only.\n\
         - Call remote_info first: it names the machine, its shell and the roots you may work in.\n\
         - Every path is relative to a remote root (the first one unless you pass `root`); never \
         an absolute path, and never a local one.\n\
         - Your local working directory is a scratch folder, not the target system: do not read, \
         edit or build there to change the remote machine.\n\
         - Use remote_run for any command that must run on the remote host."
    )
}

// ── the pending table ───────────────────────────────────────────────────────────────────────

/// How a remote call ended: the drone's output, or a sentence for the agent.
pub type Outcome = Result<RemoteOutput, String>;

/// One call waiting on a window.
struct Row {
    /// The URL segment of the agent that called — what the coordinator resolves the window from.
    caller: String,
    /// The window it was sent to, once addressed. A result from any other window is dropped.
    target: Option<ClientId>,
    reply: Sender<Outcome>,
}

/// Every remote call an agent is waiting on. Shared behind an `Arc` between the MCP listener
/// (which files and waits) and the coordinator (which addresses and releases) —
/// [`crate::browser::Commands`]'s shape.
#[derive(Default)]
pub struct RemoteCalls {
    pending: Mutex<HashMap<RemoteCallId, Row>>,
}

impl RemoteCalls {
    pub fn new() -> Self {
        Self::default()
    }

    /// File a call for the agent whose URL segment is `caller`.
    pub fn raise(&self, caller: &str) -> (RemoteCallId, Receiver<Outcome>) {
        let call_id = RemoteCallId::generate();
        let (reply, waiting) = channel();
        self.lock().insert(
            call_id,
            Row {
                caller: caller.to_string(),
                target: None,
                reply,
            },
        );
        (call_id, waiting)
    }

    /// Who filed this call, if it is still waiting.
    pub fn caller(&self, call_id: RemoteCallId) -> Option<String> {
        self.lock().get(&call_id).map(|row| row.caller.clone())
    }

    /// Note which window the call went to.
    pub fn sent_to(&self, call_id: RemoteCallId, client: ClientId) {
        if let Some(row) = self.lock().get_mut(&call_id) {
            row.target = Some(client);
        }
    }

    /// End a call with `outcome`. `false` when nothing was waiting.
    pub fn finish(&self, call_id: RemoteCallId, outcome: Outcome) -> bool {
        let Some(row) = self.lock().remove(&call_id) else {
            return false;
        };
        let _ = row.reply.send(outcome);
        true
    }

    /// A window's relayed result. Only the window the call was sent to may end it; the first
    /// result wins.
    pub fn reply(&self, client: ClientId, call_id: RemoteCallId, outcome: Outcome) -> bool {
        let mut pending = self.lock();
        match pending.get(&call_id) {
            Some(row) if row.target == Some(client) => {}
            Some(_) => {
                tracing::debug!(call = %call_id, "a remote result from a window it was not sent to");
                return false;
            }
            None => return false,
        }
        let row = pending.remove(&call_id).expect("checked above");
        drop(pending);
        let _ = row.reply.send(outcome);
        true
    }

    /// Wait for the call's end, giving up after `bound`. Called on the MCP call's own thread.
    pub fn wait(
        &self,
        call_id: RemoteCallId,
        waiting: &Receiver<Outcome>,
        bound: Duration,
    ) -> Outcome {
        match waiting.recv_timeout(bound) {
            Ok(outcome) => outcome,
            Err(RecvTimeoutError::Timeout) => {
                // A result that beat the give-up by a lock is still collected.
                if self.lock().remove(&call_id).is_none()
                    && let Ok(outcome) = waiting.try_recv()
                {
                    return outcome;
                }
                Err(format!(
                    "the remote did not answer within {}s",
                    bound.as_secs()
                ))
            }
            Err(RecvTimeoutError::Disconnected) => Err(NO_WINDOW.to_string()),
        }
    }

    /// A window has gone: everything sent to it ends as [`NO_WINDOW`].
    pub fn client_gone(&self, client: ClientId) {
        let mut pending = self.lock();
        let gone: Vec<RemoteCallId> = pending
            .iter()
            .filter(|(_, row)| row.target == Some(client))
            .map(|(call_id, _)| *call_id)
            .collect();
        for call_id in gone {
            if let Some(row) = pending.remove(&call_id) {
                let _ = row.reply.send(Err(NO_WINDOW.to_string()));
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<RemoteCallId, Row>> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

// ── the reach ───────────────────────────────────────────────────────────────────────────────

/// A claimed place among the [`MAX_CALLS`]; released on drop.
pub struct Slot(Arc<std::sync::atomic::AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl RemoteReach {
    pub fn new(calls: Arc<RemoteCalls>) -> Self {
        Self {
            calls,
            inflight: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// A place for one more call, or `None` when [`MAX_CALLS`] are running.
    pub fn claim(&self) -> Option<Slot> {
        let taken = self.inflight.fetch_add(1, Ordering::SeqCst);
        if taken >= MAX_CALLS {
            self.inflight.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Slot(Arc::clone(&self.inflight)))
    }
}

// ── the call ────────────────────────────────────────────────────────────────────────────────

pub fn call(
    tool: &str,
    arguments: &Value,
    facts: &AgentFacts,
    voice: &Voice,
    reach: &RemoteReach,
) -> Result<Value, String> {
    let Some(remote) = facts.remote.clone() else {
        return Err("this agent is not bound to a remote machine".to_string());
    };
    let project_id: ProjectId = facts
        .project
        .id
        .parse()
        .map_err(|_| "this agent is not in a project".to_string())?;
    let (root, op) = op_for(tool, arguments)?;
    let bound = match &op {
        RemoteOp::Run { timeout_ms, .. } => {
            Duration::from_millis(*timeout_ms) + Duration::from_secs(RUN_MARGIN_SECS)
        }
        _ => Duration::from_secs(TIMEOUT_SECS),
    };
    let name = op.name();
    let read_offset = match &op {
        RemoteOp::Read { offset, .. } => *offset,
        _ => 0,
    };
    let (call_id, waiting) = reach.calls.raise(&facts.key);
    tracing::debug!(agent = %facts.key, call = %call_id, op = name, "remote tool call");
    voice.say(Message::RemoteToolCall {
        call_id,
        project_id,
        remote: Some(remote),
        root,
        op,
    });
    let mutating = matches!(tool, "remote_write" | "remote_edit");
    let output = reach
        .calls
        .wait(call_id, &waiting, bound)
        .map_err(|error| {
            if mutating && error.starts_with("the remote did not answer") {
                format!(
                    "{error}; the outcome is unknown: the change may or may not have been applied. \
                     Verify with remote_read before retrying"
                )
            } else {
                error
            }
        })?;
    Ok(Value::String(format(&output, read_offset)))
}

/// A tool's arguments as the op the drone runs, and the root it names.
pub fn op_for(tool: &str, arguments: &Value) -> Result<(Option<String>, RemoteOp), String> {
    let root = optional(arguments, "root");
    let op = match tool {
        "remote_info" => RemoteOp::Info,
        "remote_list" => RemoteOp::List {
            path: optional(arguments, "path").unwrap_or_default(),
            depth: arguments
                .get("depth")
                .and_then(Value::as_u64)
                .map(|depth| depth.min(u64::from(u32::MAX)) as u32),
        },
        "remote_read" => RemoteOp::Read {
            path: required(arguments, "path")?,
            offset: number(arguments, "offset").unwrap_or(0),
            limit: number(arguments, "limit").unwrap_or(0),
        },
        "remote_write" => RemoteOp::Write {
            path: required(arguments, "path")?,
            content: text(arguments, "content")?,
            overwrite: flag(arguments, "overwrite"),
        },
        "remote_edit" => RemoteOp::Edit {
            path: required(arguments, "path")?,
            old: arguments
                .get("old")
                .and_then(Value::as_str)
                .filter(|old| !old.is_empty())
                .map(str::to_string)
                .ok_or_else(|| "needs a non-empty 'old' argument".to_string())?,
            new: text(arguments, "new")?,
            replace_all: flag(arguments, "replace_all"),
        },
        "remote_glob" => RemoteOp::Glob {
            pattern: required(arguments, "pattern")?,
            path: optional(arguments, "path"),
        },
        "remote_grep" => RemoteOp::Grep {
            pattern: required(arguments, "pattern")?,
            path: optional(arguments, "path"),
            glob: optional(arguments, "glob"),
            case_insensitive: flag(arguments, "case_insensitive"),
            max_results: number(arguments, "max_results")
                .map(|max| max.clamp(1, u64::from(GREP_MAX)) as u32)
                .unwrap_or(GREP_DEFAULT),
        },
        "remote_run" => RemoteOp::Run {
            cmd: required(arguments, "cmd")?,
            cwd: optional(arguments, "cwd"),
            timeout_ms: number(arguments, "timeout_ms")
                .unwrap_or(RUN_DEFAULT_MS)
                .clamp(1_000, RUN_MAX_MS),
            shell: optional(arguments, "shell"),
        },
        _ => return Err(format!("unknown tool: ubiq-remote/{tool}")),
    };
    Ok((root, op))
}

// ── the answer ──────────────────────────────────────────────────────────────────────────────

/// A drone's output as compact text for a model. `read_offset` is the line a read started at, so
/// its lines are numbered as the file numbers them; every other op ignores it.
pub fn format(output: &RemoteOutput, read_offset: u64) -> String {
    let mut out = String::new();
    match output {
        RemoteOutput::Info(info) => info_text(&mut out, info),
        RemoteOutput::List { entries, truncated } => {
            if entries.is_empty() {
                out.push_str("(empty)\n");
            }
            for entry in entries {
                match entry.kind {
                    EntryKind::Dir => {
                        let _ = writeln!(out, "{}/", entry.path.trim_end_matches('/'));
                    }
                    EntryKind::File => match entry.size {
                        Some(size) => {
                            let _ = writeln!(out, "{}  {size}", entry.path);
                        }
                        None => {
                            let _ = writeln!(out, "{}", entry.path);
                        }
                    },
                    EntryKind::Other => {
                        let _ = writeln!(out, "{}  (other)", entry.path);
                    }
                }
            }
            if *truncated {
                out.push_str("[truncated: list a narrower path or a smaller depth]\n");
            }
        }
        RemoteOutput::Read { content, truncated } => {
            read_text(&mut out, content, read_offset, *truncated)
        }
        RemoteOutput::Write { bytes, created } => {
            let _ = writeln!(
                out,
                "wrote {bytes} bytes ({})",
                if *created { "created" } else { "overwritten" }
            );
        }
        RemoteOutput::Edit { replacements } => {
            let _ = writeln!(
                out,
                "{replacements} replacement{}",
                if *replacements == 1 { "" } else { "s" }
            );
        }
        RemoteOutput::Glob { paths, truncated } => {
            if paths.is_empty() {
                out.push_str("no matches\n");
            }
            for path in paths {
                let _ = writeln!(out, "{path}");
            }
            if *truncated {
                out.push_str("[truncated: narrow the pattern or the path]\n");
            }
        }
        RemoteOutput::Grep { matches, truncated } => {
            if matches.is_empty() {
                out.push_str("no matches\n");
            }
            for hit in matches {
                let _ = writeln!(out, "{}:{}: {}", hit.path, hit.line, hit.text);
            }
            if *truncated {
                out.push_str("[truncated: raise max_results or narrow the search]\n");
            }
        }
        RemoteOutput::Run(run) => run_text(&mut out, run),
    }
    out
}

fn info_text(out: &mut String, info: &RemoteInfo) {
    let _ = writeln!(out, "host: {} ({}/{})", info.hostname, info.os, info.arch);
    if let Some(shell) = &info.shell {
        let _ = writeln!(out, "shell: {shell}");
    }
    out.push_str("roots (paths are relative to one; the first is the default):\n");
    for root in &info.roots {
        let _ = writeln!(
            out,
            "  {}  {}{}",
            root.name,
            root.path,
            if root.read_only { "  [read-only]" } else { "" }
        );
    }
    if !info.tools.is_empty() {
        let tools: Vec<String> = info
            .tools
            .iter()
            .map(|(name, version)| {
                if version.is_empty() {
                    name.clone()
                } else {
                    format!("{name} {version}")
                }
            })
            .collect();
        let _ = writeln!(out, "tools: {}", tools.join(", "));
    }
}

fn read_text(out: &mut String, content: &str, offset: u64, truncated: bool) {
    let mut count = 0u64;
    for (index, line) in content.lines().enumerate() {
        let _ = writeln!(out, "{:>6}\t{line}", offset + index as u64 + 1);
        count += 1;
    }
    if count == 0 {
        out.push_str("(no lines)\n");
    }
    if truncated {
        let _ = writeln!(out, "[truncated: continue with offset={}]", offset + count);
    }
}

fn run_text(out: &mut String, run: &RemoteRun) {
    match (run.exit_code, run.timed_out) {
        (_, true) => out.push_str("exit: timed out, killed\n"),
        (Some(code), false) => {
            let _ = writeln!(out, "exit: {code}");
        }
        (None, false) => out.push_str("exit: killed by a signal\n"),
    }
    for (name, stream) in [("stdout", &run.stdout), ("stderr", &run.stderr)] {
        if stream.is_empty() {
            continue;
        }
        let _ = writeln!(out, "--- {name} ---");
        out.push_str(stream);
        if !stream.ends_with('\n') {
            out.push('\n');
        }
    }
    if run.truncated {
        out.push_str("[output truncated: only the tail of each stream is kept]\n");
    }
}

fn required(arguments: &Value, key: &str) -> Result<String, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("needs a non-empty '{key}' argument"))
}

/// A string argument that must be present but may be empty — a file's content, an edit's `new`.
fn text(arguments: &Value, key: &str) -> Result<String, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("needs a '{key}' argument"))
}

fn optional(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn number(arguments: &Value, key: &str) -> Option<u64> {
    arguments.get(key).and_then(Value::as_u64)
}

fn flag(arguments: &Value, key: &str) -> bool {
    arguments.get(key).and_then(Value::as_bool).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use ubiq_proto::remote_tools::{RemoteEntry, RemoteMatch, RemoteRoot};

    #[test]
    fn each_tool_maps_to_its_op_and_root() {
        assert_eq!(
            op_for("remote_info", &json!({})).unwrap(),
            (None, RemoteOp::Info)
        );
        assert_eq!(
            op_for(
                "remote_read",
                &json!({"path": "src/main.rs", "offset": 10, "root": "app"})
            )
            .unwrap(),
            (
                Some("app".into()),
                RemoteOp::Read {
                    path: "src/main.rs".into(),
                    offset: 10,
                    limit: 0
                }
            )
        );
        assert_eq!(
            op_for("remote_list", &json!({})).unwrap().1,
            RemoteOp::List {
                path: String::new(),
                depth: None
            }
        );
        assert_eq!(
            op_for("remote_write", &json!({"path": "a", "content": ""}))
                .unwrap()
                .1,
            RemoteOp::Write {
                path: "a".into(),
                content: String::new(),
                overwrite: false
            }
        );
        assert_eq!(
            op_for(
                "remote_edit",
                &json!({"path": "a", "old": "x", "new": "", "replace_all": true})
            )
            .unwrap()
            .1,
            RemoteOp::Edit {
                path: "a".into(),
                old: "x".into(),
                new: String::new(),
                replace_all: true
            }
        );
        assert_eq!(
            op_for(
                "remote_grep",
                &json!({"pattern": "fn", "max_results": 999_999})
            )
            .unwrap()
            .1,
            RemoteOp::Grep {
                pattern: "fn".into(),
                path: None,
                glob: None,
                case_insensitive: false,
                max_results: GREP_MAX
            }
        );
        assert_eq!(
            op_for("remote_glob", &json!({"pattern": "**/*.rs", "path": "src"}))
                .unwrap()
                .1,
            RemoteOp::Glob {
                pattern: "**/*.rs".into(),
                path: Some("src".into())
            }
        );
        assert_eq!(
            op_for("remote_run", &json!({"cmd": "make", "timeout_ms": 5}))
                .unwrap()
                .1,
            RemoteOp::Run {
                cmd: "make".into(),
                cwd: None,
                timeout_ms: 1_000,
                shell: None
            }
        );
    }

    #[test]
    fn a_missing_argument_or_an_unknown_tool_is_refused() {
        assert!(op_for("remote_read", &json!({})).is_err());
        assert!(op_for("remote_write", &json!({"path": "a"})).is_err());
        assert!(op_for("remote_edit", &json!({"path": "a", "old": "", "new": "b"})).is_err());
        assert!(op_for("remote_run", &json!({"cmd": "  "})).is_err());
        assert!(op_for("remote_nope", &json!({})).is_err());
    }

    #[test]
    fn a_read_is_numbered_from_its_offset_and_says_where_to_continue() {
        let output = RemoteOutput::Read {
            content: "one\ntwo\n".into(),
            truncated: true,
        };
        assert_eq!(
            format(&output, 10),
            "    11\tone\n    12\ttwo\n[truncated: continue with offset=12]\n"
        );
    }

    #[test]
    fn a_run_is_its_exit_and_its_non_empty_streams() {
        let run = RemoteOutput::Run(Box::new(RemoteRun {
            exit_code: Some(2),
            stdout: String::new(),
            stderr: "error: nope".into(),
            truncated: true,
            timed_out: false,
        }));
        assert_eq!(
            format(&run, 0),
            "exit: 2\n--- stderr ---\nerror: nope\n[output truncated: only the tail of each stream is kept]\n"
        );
        let killed = RemoteOutput::Run(Box::new(RemoteRun {
            exit_code: None,
            stdout: "ok\n".into(),
            stderr: String::new(),
            truncated: false,
            timed_out: true,
        }));
        assert_eq!(
            format(&killed, 0),
            "exit: timed out, killed\n--- stdout ---\nok\n"
        );
    }

    #[test]
    fn info_list_and_grep_are_one_line_per_fact() {
        let info = RemoteOutput::Info(Box::new(RemoteInfo {
            os: "linux".into(),
            arch: "x86_64".into(),
            hostname: "box".into(),
            shell: Some("/bin/bash".into()),
            roots: vec![RemoteRoot {
                name: "home".into(),
                path: "/home/me".into(),
                read_only: true,
            }],
            tools: vec![("rg".into(), "14.1".into())],
        }));
        assert_eq!(
            format(&info, 0),
            "host: box (linux/x86_64)\nshell: /bin/bash\nroots (paths are relative to one; the first is the default):\n  home  /home/me  [read-only]\ntools: rg 14.1\n"
        );
        let list = RemoteOutput::List {
            entries: vec![
                RemoteEntry {
                    path: "src".into(),
                    kind: EntryKind::Dir,
                    size: None,
                },
                RemoteEntry {
                    path: "a.txt".into(),
                    kind: EntryKind::File,
                    size: Some(5),
                },
            ],
            truncated: false,
        };
        assert_eq!(format(&list, 0), "src/\na.txt  5\n");
        let grep = RemoteOutput::Grep {
            matches: vec![RemoteMatch {
                path: "a.rs".into(),
                line: 3,
                text: "fn main()".into(),
            }],
            truncated: false,
        };
        assert_eq!(format(&grep, 0), "a.rs:3: fn main()\n");
    }

    #[test]
    fn a_label_becomes_one_safe_segment() {
        assert_eq!(sanitise("build box #1"), "build-box--1");
        assert_eq!(sanitise("../etc"), "etc");
        assert_eq!(sanitise("  "), "remote");
        assert_eq!(
            workdir(Path::new("/p"), "a/b"),
            Path::new("/p/.ubiq/remote/a-b")
        );
        assert_eq!(
            drone_workdir(Path::new("/cfg/projects/x/local/remote"), "win box"),
            Path::new("/cfg/projects/x/local/remote/win-box")
        );
    }

    #[test]
    fn a_result_from_the_window_it_was_sent_to_ends_the_call() {
        let (hub, _host) = ubiq_proto::bus::hub();
        let (one, two) = (hub.connect().id(), hub.connect().id());
        let calls = RemoteCalls::new();
        let (call_id, waiting) = calls.raise("agent");
        assert_eq!(calls.caller(call_id).as_deref(), Some("agent"));
        calls.sent_to(call_id, one);
        assert!(!calls.reply(two, call_id, Err("x".into())));
        assert!(calls.reply(one, call_id, Ok(RemoteOutput::Edit { replacements: 1 })));
        assert_eq!(
            calls.wait(call_id, &waiting, Duration::from_millis(5)),
            Ok(RemoteOutput::Edit { replacements: 1 })
        );
        // A second result finds nothing.
        assert!(!calls.reply(one, call_id, Err("late".into())));
    }

    #[test]
    fn a_wait_nobody_answers_times_out_and_a_gone_window_releases() {
        let calls = RemoteCalls::new();
        let (call_id, waiting) = calls.raise("agent");
        let outcome = calls.wait(call_id, &waiting, Duration::from_millis(5));
        assert!(outcome.unwrap_err().contains("did not answer"));
        assert!(calls.caller(call_id).is_none());

        let (hub, _host) = ubiq_proto::bus::hub();
        let window = hub.connect().id();
        let (call_id, waiting) = calls.raise("agent");
        calls.sent_to(call_id, window);
        calls.client_gone(window);
        assert_eq!(
            calls.wait(call_id, &waiting, Duration::from_millis(5)),
            Err(NO_WINDOW.to_string())
        );
    }
}
