//! opencode's NDJSON bridge — a concrete [`super::IoBridge`] implementation.
//!
//! Speaks the wire protocol documented in `_docs/harness/opencode.md`
//! §"Orchestration / headless invocation" / §"Output stream protocol": one
//! JSON object per line on stdout (events only; opencode is **one-shot**, with
//! the prompt delivered via argv at launch, not over stdin). No input stream
//! exists.
//!
//! This is **core** (always compiled, no feature gate): only `std::process`,
//! `std::sync`, `std::thread`, `serde_json` and `tracing` are used, matching
//! [`super::structured`]'s "no pty, no clap" discipline so a lib-mode
//! embedder can use it without the `pty`/`cli` features.
//!
//! ## Design
//!
//! [`OpencodeBridge::new`] takes ownership of a spawned [`std::process::Child`]
//! (from [`super::spawn_piped`]), splits off its stdout, and spawns a
//! dedicated **reader thread** that scans stdout line-by-line and pushes mapped
//! [`AgentEvent`]s onto an `mpsc` channel. stdin is dropped immediately since
//! opencode is one-shot and accepts no further input (the prompt is part of
//! the argv). On stdout EOF, the reader thread emits a terminal
//! `AgentEvent::TurnEnded` (if not already sent by an explicit `error` event)
//! and closes the channel.
//!
//! The same reader-thread architecture as [`super::jsonl`] prevents blocking
//! on writes, even though opencode takes no stdin input: the child might
//! buffer output, and keeping the reader draining stdout prevents that from
//! stalling the process (a full pipe blocks the producer).
//!
//! ## What opencode never reports
//!
//! `step_start` carries only a session id — no model, mode, tool list or
//! subagent list, so [`AgentEvent::SessionStarted`] leaves those `None`/empty.
//! `step_finish.part.tokens` gives per-turn input/output counts but no
//! context window, and a ratio with an invented denominator is worse than no
//! ratio — so this bridge never emits [`AgentEvent::UsageUpdate`] at all.
//! There is also no on-stream approval handshake (opencode runs headless with
//! `--dangerously-skip-permissions`), so [`AgentEvent::PermissionRequest`]
//! never appears either.
//!
//! ## Logging
//!
//! Every raw line is a `trace!`; every mapped event is a `debug!`. Raw frames
//! carry prompts and file contents, which is why they sit a level below
//! everything else.

use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdout};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{
    AgentEvent, AgentInput, Content, IoBridge, StopReason, ToolCall, ToolCallUpdate, ToolContent,
    ToolKind, ToolLocation, ToolStatus,
};

/// How long [`Drop`] waits for the child to exit after the reader thread
/// finishes draining stdout before killing it. Mirrors
/// `_docs/harness/opencode.md` §"Process lifecycle".
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// A live bridge to an opencode process running headlessly with
/// `--format json` (one-shot NDJSON on stdout, no input stream).
pub struct OpencodeBridge {
    child: Child,
    events: mpsc::Receiver<AgentEvent>,
    reader: Option<std::thread::JoinHandle<()>>,
    /// Whether a terminal `TurnEnded` has already been emitted via the stream
    /// (an explicit error), so EOF doesn't send a second one.
    /// Shared with the reader thread via `Arc<Mutex<bool>>`.
    #[allow(dead_code)]
    turn_ended: Arc<Mutex<bool>>,
}

impl OpencodeBridge {
    /// Wrap an already-spawned opencode child (from [`super::spawn_piped`])
    /// as a [`OpencodeBridge`].
    ///
    /// Takes ownership of the child and its stdout; stdin is immediately
    /// dropped (opencode is one-shot). Returns an error if stdout is not
    /// piped (a programmer error — [`super::spawn_piped`] always pipes both).
    pub fn new(mut child: Child) -> crate::Result<Self> {
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("child stdout is not piped"))?;

        // Drop stdin immediately; opencode takes no input.
        let _ = child.stdin.take();

        let (tx, rx) = mpsc::channel();
        let turn_ended = Arc::new(Mutex::new(false));
        let turn_ended_clone = Arc::clone(&turn_ended);

        let reader = std::thread::spawn(move || read_loop(stdout, tx, turn_ended_clone));

        Ok(Self {
            child,
            events: rx,
            reader: Some(reader),
            turn_ended,
        })
    }
}

impl IoBridge for OpencodeBridge {
    fn send(&mut self, input: AgentInput) -> crate::Result<()> {
        match input {
            AgentInput::Prompt { .. } => {
                // opencode is one-shot: the prompt is delivered via argv at
                // launch. Sending a prompt on the bridge is a no-op.
                Ok(())
            }
            AgentInput::AnswerPermission { .. } => {
                // opencode runs headless with `--dangerously-skip-permissions`,
                // so no `PermissionRequest` is ever emitted for this to
                // answer. A no-op rather than an error: there is nothing
                // wrong with the caller's intent, just nothing waiting.
                Ok(())
            }
            AgentInput::Cancel => {
                // Best-effort signal: kill the process so the run stops.
                let _ = self.child.kill();
                Ok(())
            }
            AgentInput::SetConfigOption { config_id, .. } => Err(anyhow::anyhow!(
                "opencode's one-shot bridge cannot change '{config_id}' mid-session"
            )),
        }
    }

    fn next_event(&mut self) -> crate::Result<Option<AgentEvent>> {
        match self.events.recv() {
            Ok(ev) => Ok(Some(ev)),
            // Sender dropped == reader thread exited == stdout hit EOF.
            Err(mpsc::RecvError) => Ok(None),
        }
    }

    // `input()` keeps the default `None`: opencode takes no input after
    // launch (the prompt is argv-only), so there is no sink to hand a caller.
}

impl Drop for OpencodeBridge {
    fn drop(&mut self) {
        // The reader thread owns stdout and will exit once it hits EOF.
        // Wait a bounded time for the child to exit naturally, then kill it
        // if needed.
        let deadline = Instant::now() + DRAIN_TIMEOUT;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        // NOTE: SIGTERM-then-SIGKILL process-group teardown
                        // deferred (unsafe-free constraint: portable-pty is not
                        // available in this core module).
                        let _ = self.child.kill();
                        let _ = self.child.wait();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }

        // Join the reader thread (stdout EOF unblocks it).
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// The reader thread body: scan stdout line-by-line (NDJSON), map each line
/// to zero-or-more [`AgentEvent`]s and push them onto `tx`. On stream end,
/// emit a terminal [`AgentEvent::TurnEnded`] if one hasn't been sent already
/// (no explicit error), then drop `tx` (closing the channel).
fn read_loop(stdout: ChildStdout, tx: mpsc::Sender<AgentEvent>, turn_ended: Arc<Mutex<bool>>) {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        tracing::trace!(direction = "in", frame = %line, "opencode ndjson");
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            // Not a recognized JSON line — ignore.
            continue;
        };

        for ev in map_event(&value) {
            tracing::debug!(event = ?ev, "opencode event");
            if matches!(ev, AgentEvent::TurnEnded { .. })
                && let Ok(mut ended) = turn_ended.lock()
            {
                *ended = true;
            }
            if tx.send(ev).is_err() {
                // No one is listening anymore.
                return;
            }
        }
    }

    // EOF: emit a successful turn end if the stream didn't already end one.
    if let Ok(mut ended) = turn_ended.lock()
        && !*ended
    {
        let ev = AgentEvent::TurnEnded {
            stop_reason: StopReason::EndTurn,
            error: None,
        };
        tracing::debug!(event = ?ev, "opencode event");
        let _ = tx.send(ev);
        *ended = true;
    }
}

/// Map one parsed NDJSON stdout line to zero or more [`AgentEvent`]s.
///
/// Pure (no I/O) so it's unit-testable directly; see the mapping table in
/// `_docs/harness/opencode.md` §"Output stream protocol".
fn map_event(value: &Value) -> Vec<AgentEvent> {
    match value.get("type").and_then(Value::as_str) {
        Some("step_start") => {
            vec![AgentEvent::SessionStarted {
                session_id: value
                    .get("sessionID")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                // opencode's step_start carries only the session id.
                model: None,
                mode: None,
                tools: Vec::new(),
                agents: Vec::new(),
            }]
        }
        Some("text") => {
            if let Some(text) = value
                .get("part")
                .and_then(|p| p.get("text"))
                .and_then(Value::as_str)
            {
                vec![AgentEvent::AgentMessageChunk {
                    content: Content::text(text),
                    // opencode doesn't tag a part with a message id.
                    message_id: None,
                }]
            } else {
                Vec::new()
            }
        }
        Some("tool_use") => map_tool_use(value),
        Some("step_finish") => {
            // `part.tokens` gives per-turn input/output counts, but never a
            // context window — and a ratio with an invented denominator is
            // worse than no ratio, so no `UsageUpdate` is emitted here.
            Vec::new()
        }
        Some("error") => {
            let message = value
                .get("error")
                .and_then(|e| e.get("data"))
                .and_then(|d| d.get("message"))
                .and_then(Value::as_str)
                .or_else(|| value.get("error").and_then(Value::as_str))
                .map(str::to_string)
                .unwrap_or_else(|| "unknown error".to_string());

            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::Failed,
                error: Some(message),
            }]
        }
        other => {
            tracing::trace!(kind = ?other, "opencode event ignored");
            Vec::new()
        }
    }
}

/// A `tool_use` part: opencode carries the call's input and (once finished)
/// its output in the same `state` object, so one line can yield both a
/// [`AgentEvent::ToolCall`] and its [`AgentEvent::ToolCallUpdate`].
fn map_tool_use(value: &Value) -> Vec<AgentEvent> {
    let Some(part) = value.get("part") else {
        return Vec::new();
    };
    let tool_name = part.get("tool").and_then(Value::as_str).unwrap_or("tool");
    let call_id = part.get("callID").and_then(Value::as_str);

    let Some(state) = part.get("state") else {
        return Vec::new();
    };
    let input = state.get("input").cloned().unwrap_or(Value::Null);
    let status = state.get("status").and_then(Value::as_str).unwrap_or("");

    let call = build_tool_call(tool_name, call_id, &input);
    let id = call.id.clone();
    let mut events = vec![AgentEvent::ToolCall { call }];

    if status == "complete" {
        let output = state.get("output").cloned().unwrap_or(Value::Null);
        events.push(AgentEvent::ToolCallUpdate {
            update: ToolCallUpdate {
                content: output.as_str().map(|text| {
                    vec![ToolContent::Content {
                        content: Content::text(text),
                    }]
                }),
                raw_output: Some(output),
                ..ToolCallUpdate::finished(id, ToolStatus::Completed)
            },
        });
    }

    events
}

/// A `tool_use` part's call, before it has finished. Field names
/// (`filePath`/`path`/`command`/`pattern`/`url`) are the ones opencode's
/// built-in tools are documented to use; a custom or MCP tool with a
/// different input shape still gets a call, just without a target in the
/// title or a location.
fn build_tool_call(name: &str, call_id: Option<&str>, input: &Value) -> ToolCall {
    let string = |key: &str| input.get(key).and_then(Value::as_str);
    let path = string("filePath").or_else(|| string("path"));
    let target = path
        .or_else(|| string("command"))
        .or_else(|| string("pattern"))
        .or_else(|| string("url"));

    let title = match target {
        Some(target) => format!("{name} {target}"),
        None => name.to_string(),
    };

    let mut call = ToolCall::new(call_id.unwrap_or(name), title);
    call.kind = tool_kind(name);
    call.status = ToolStatus::InProgress;
    call.raw_input = Some(input.clone());
    if let Some(path) = path {
        call.locations = vec![ToolLocation {
            path: path.to_string(),
            line: None,
        }];
    }
    call
}

/// opencode's built-in tool names onto the ten kinds a consumer draws
/// (`_docs/harness/opencode.md` §"Recognised permission keys"). An unknown
/// name — a custom tool or an MCP tool — is [`ToolKind::Other`] rather than a
/// guess.
fn tool_kind(name: &str) -> ToolKind {
    match name {
        "read" | "glob" | "list" => ToolKind::Read,
        "edit" | "write" | "apply_patch" => ToolKind::Edit,
        "bash" => ToolKind::Execute,
        "grep" | "websearch" => ToolKind::Search,
        "webfetch" => ToolKind::Fetch,
        "task" | "todowrite" | "todoread" | "question" => ToolKind::Think,
        _ => ToolKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn map_event_step_start_is_session_started() {
        let v = json!({"type":"step_start","sessionID":"sess-123"});
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::SessionStarted {
                session_id: Some("sess-123".to_string()),
                model: None,
                mode: None,
                tools: Vec::new(),
                agents: Vec::new(),
            }]
        );
    }

    #[test]
    fn map_event_text_is_agent_message_chunk() {
        let v = json!({"type":"text","part":{"text":"hello world"}});
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::AgentMessageChunk {
                content: Content::text("hello world"),
                message_id: None,
            }]
        );
    }

    #[test]
    fn map_event_tool_use_complete_emits_call_and_update() {
        let v = json!({
            "type":"tool_use",
            "part":{
                "tool":"bash",
                "callID":"call-1",
                "state":{
                    "status":"complete",
                    "input":{"command":"ls"},
                    "output":"file1.txt\nfile2.txt"
                }
            }
        });
        let events = map_event(&v);
        assert_eq!(events.len(), 2);
        let AgentEvent::ToolCall { call } = &events[0] else {
            panic!("expected a tool call, got {:?}", events[0]);
        };
        assert_eq!(call.id, "call-1");
        assert_eq!(call.title, "bash ls");
        assert_eq!(call.kind, ToolKind::Execute);
        assert_eq!(call.status, ToolStatus::InProgress);

        let AgentEvent::ToolCallUpdate { update } = &events[1] else {
            panic!("expected an update, got {:?}", events[1]);
        };
        assert_eq!(update.id, "call-1");
        assert_eq!(update.status, Some(ToolStatus::Completed));
        assert_eq!(
            update.content,
            Some(vec![ToolContent::Content {
                content: Content::text("file1.txt\nfile2.txt"),
            }])
        );
    }

    #[test]
    fn map_event_tool_use_incomplete_emits_call_only() {
        let v = json!({
            "type":"tool_use",
            "part":{
                "tool":"bash",
                "callID":"call-1",
                "state":{
                    "status":"pending",
                    "input":{"command":"sleep 10"},
                    "output":null
                }
            }
        });
        let events = map_event(&v);
        assert_eq!(events.len(), 1);
        let AgentEvent::ToolCall { call } = &events[0] else {
            panic!("expected a tool call, got {:?}", events[0]);
        };
        assert_eq!(call.id, "call-1");
        assert_eq!(call.status, ToolStatus::InProgress);
    }

    /// A file-touching tool carries a location, so a consumer can follow
    /// along without parsing `raw_input` itself.
    #[test]
    fn map_event_tool_use_edit_carries_a_location() {
        let v = json!({
            "type":"tool_use",
            "part":{
                "tool":"edit",
                "callID":"call-2",
                "state":{
                    "status":"pending",
                    "input":{"filePath":"/tmp/a.rs"}
                }
            }
        });
        let events = map_event(&v);
        let AgentEvent::ToolCall { call } = &events[0] else {
            panic!("expected a tool call");
        };
        assert_eq!(call.kind, ToolKind::Edit);
        assert_eq!(
            call.locations,
            vec![ToolLocation {
                path: "/tmp/a.rs".to_string(),
                line: None,
            }]
        );
    }

    /// `step_finish` carries tokens but never a context window, so no ratio
    /// can be reported — the honest thing is no event at all.
    #[test]
    fn map_event_step_finish_emits_nothing() {
        let v = json!({
            "type":"step_finish",
            "part":{
                "tokens":{
                    "input":150,
                    "output":200,
                    "cache":{"read":0,"write":0}
                }
            }
        });
        assert_eq!(map_event(&v), Vec::new());
    }

    #[test]
    fn map_event_error_ends_the_turn_as_failed() {
        let v = json!({
            "type":"error",
            "error":{
                "name":"UnknownError",
                "data":{"message":"something went wrong"}
            }
        });
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::Failed,
                error: Some("something went wrong".to_string()),
            }]
        );
    }

    #[test]
    fn map_event_unknown_type_is_ignored() {
        let v = json!({"type":"something_new","foo":"bar"});
        assert_eq!(map_event(&v), Vec::new());
    }

    #[test]
    fn map_event_text_without_text_field_is_ignored() {
        let v = json!({"type":"text","part":{}});
        assert_eq!(map_event(&v), Vec::new());
    }

    #[test]
    fn map_event_tool_use_without_state_is_ignored() {
        let v = json!({"type":"tool_use","part":{"tool":"bash","callID":"c1"}});
        assert_eq!(map_event(&v), Vec::new());
    }
}
