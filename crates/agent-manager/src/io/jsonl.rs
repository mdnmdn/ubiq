//! Claude Code's `stream-json` (NDJSON) bridge — the first concrete
//! [`super::IoBridge`] implementation.
//!
//! Speaks the wire protocol documented in `_docs/harness/claude-code.md`
//! §"Output stream protocol" / §"Tool approval in headless mode": one JSON
//! object per line on stdout (events), one JSON object per line on stdin
//! (prompts and `control_response` answers).
//!
//! This is **core** (always compiled, no feature gate): only `std::process`,
//! `std::sync`, `std::thread`, and `serde_json` are used, matching
//! [`super::structured`]'s "no pty, no clap" discipline so a lib-mode
//! embedder can use it without the `pty`/`cli` features.
//!
//! ## Design
//!
//! [`JsonlBridge::new`] takes ownership of a spawned [`std::process::Child`]
//! (from [`super::spawn_piped`]), splits off its stdin/stdout, and spawns a
//! dedicated **reader thread** that scans stdout line-by-line and pushes
//! mapped [`AgentEvent`]s onto an `mpsc` channel. This mirrors the P1 lesson
//! from the PTY runner ([`crate::run`]): the consumer of a process's stdout
//! must always be draining it on its own thread, independent of when the
//! bridge owner calls [`JsonlBridge::send`] — otherwise a prompt write on
//! [`IoBridge::send`] could block forever waiting for stdout to be drained
//! (a full pipe buffer stalls the child, which stalls the write... but
//! nobody is reading because the same thread is busy writing).
//!
//! stdin is shared as `Arc<Mutex<Option<ChildStdin>>>` because *two*
//! producers write to it: [`JsonlBridge::send`] (from the bridge owner's
//! thread) and the reader thread itself (auto-allow `control_response`
//! lines, written the moment a `control_request` is scanned off stdout, so
//! an unattended run makes progress without a consumer answering). Wrapping
//! it in `Option` (rather than just `Mutex<ChildStdin>`) gives
//! [`AgentInput::Interrupt`] and [`Drop`] a way to *close* stdin (drop it)
//! while it's shared.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{AgentEvent, AgentInput, ApprovalDecision, IoBridge};

/// How long [`Drop`] waits for the child to exit after closing stdin before
/// giving up and killing it. Mirrors `_docs/harness/claude-code.md`
/// §"Process lifecycle": "allow ~10s for the process to drain before
/// killing".
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// A live bridge to a Claude Code process speaking `stream-json` on
/// stdin/stdout (`-p --output-format stream-json --input-format
/// stream-json`).
pub struct JsonlBridge {
    child: Child,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    events: mpsc::Receiver<AgentEvent>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl JsonlBridge {
    /// Wrap an already-spawned Claude Code child (piped stdin/stdout, e.g.
    /// from [`super::spawn_piped`]) as a [`JsonlBridge`].
    ///
    /// Errors if `child`'s stdin/stdout aren't piped (a programmer error —
    /// [`super::spawn_piped`] always pipes both).
    pub fn new(mut child: Child) -> crate::Result<Self> {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("child stdin is not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("child stdout is not piped"))?;

        let stdin = Arc::new(Mutex::new(Some(stdin)));
        let (tx, rx) = mpsc::channel();

        let reader_stdin = Arc::clone(&stdin);
        let reader = std::thread::spawn(move || read_loop(stdout, reader_stdin, tx));

        Ok(Self {
            child,
            stdin,
            events: rx,
            reader: Some(reader),
        })
    }
}

impl IoBridge for JsonlBridge {
    fn send(&mut self, input: AgentInput) -> crate::Result<()> {
        match input {
            AgentInput::Prompt { text } => {
                let line = json!({
                    "type": "user",
                    "message": {
                        "role": "user",
                        "content": [{"type": "text", "text": text}],
                    },
                });
                write_line(&self.stdin, &line)
            }
            AgentInput::ApproveTool {
                request_id,
                decision,
                updated_input,
            } => {
                let behavior = match decision {
                    ApprovalDecision::Allow => "allow",
                    ApprovalDecision::Deny => "deny",
                };
                let line = control_response(
                    &request_id,
                    behavior,
                    updated_input.unwrap_or_else(|| json!({})),
                );
                write_line(&self.stdin, &line)
            }
            AgentInput::Interrupt => {
                // Close stdin so Claude Code sees EOF and stops; the reader
                // thread keeps draining stdout until the process actually
                // exits (see `_docs/harness/claude-code.md`
                // §"Process lifecycle").
                if let Ok(mut guard) = self.stdin.lock() {
                    *guard = None;
                }
                Ok(())
            }
        }
    }

    fn next_event(&mut self) -> crate::Result<Option<AgentEvent>> {
        match self.events.recv() {
            Ok(ev) => Ok(Some(ev)),
            // Sender dropped == reader thread exited == stdout hit EOF.
            Err(mpsc::RecvError) => Ok(None),
        }
    }
}

impl Drop for JsonlBridge {
    fn drop(&mut self) {
        // Close stdin first (best-effort signal to stop), then give the
        // child a bounded window to drain/exit before killing it.
        if let Ok(mut guard) = self.stdin.lock() {
            *guard = None;
        }

        let deadline = Instant::now() + DRAIN_TIMEOUT;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = self.child.kill();
                        let _ = self.child.wait();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }

        // Stdout hits EOF once the child has actually exited, which unblocks
        // the reader thread's scan loop.
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// The reader thread body: scan `stdout` line-by-line (NDJSON), map each
/// line to zero-or-more [`AgentEvent`]s and push them onto `tx`, and
/// auto-allow any `control_request` by writing a `control_response` to
/// `stdin` (shared with [`JsonlBridge::send`]).
///
/// Returns (and drops `tx`, closing the channel) on stdout EOF, a channel
/// disconnect (nobody left to receive), or a stdin lock failure.
fn read_loop(
    stdout: ChildStdout,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    tx: mpsc::Sender<AgentEvent>,
) {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            // Not a recognized JSON line — ignore rather than error, per
            // the mapping contract ("don't error on unrecognized lines").
            continue;
        };

        let request_id = is_control_request(&value)
            .then(|| value.get("request_id").and_then(Value::as_str))
            .flatten()
            .map(str::to_string);

        for ev in map_event(&value) {
            if tx.send(ev).is_err() {
                // No one is listening anymore.
                return;
            }
        }

        if let Some(request_id) = request_id {
            let response = control_response(&request_id, "allow", json!({}));
            if write_line(&stdin, &response).is_err() {
                return;
            }
        }
    }
}

/// `true` if `value` is a `{"type":"control_request",...}` event.
fn is_control_request(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("control_request")
}

/// Build the `control_response` NDJSON line
/// (`_docs/harness/claude-code.md` §"Tool approval in headless mode").
fn control_response(request_id: &str, behavior: &str, updated_input: Value) -> Value {
    json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": {
                "behavior": behavior,
                "updatedInput": updated_input,
            },
        },
    })
}

/// Serialize `value` as one NDJSON line and write it to the shared stdin,
/// under the shared lock. A `None` stdin (closed, e.g. after
/// [`AgentInput::Interrupt`]) is a silent no-op rather than an error — the
/// process is already being told to stop.
fn write_line(stdin: &Arc<Mutex<Option<ChildStdin>>>, value: &Value) -> crate::Result<()> {
    let mut guard = stdin
        .lock()
        .map_err(|_| anyhow::anyhow!("jsonl bridge stdin lock poisoned"))?;
    if let Some(stdin) = guard.as_mut() {
        writeln!(stdin, "{value}")?;
        stdin.flush()?;
    }
    Ok(())
}

/// Map one parsed NDJSON stdout line to zero or more [`AgentEvent`]s.
///
/// Pure (no I/O) so it's unit-testable directly; see the mapping table in
/// `_docs/harness/claude-code.md` §"Output stream protocol".
fn map_event(value: &Value) -> Vec<AgentEvent> {
    match value.get("type").and_then(Value::as_str) {
        Some("system") if value.get("subtype").and_then(Value::as_str) == Some("init") => {
            vec![AgentEvent::SessionStarted {
                session_id: value
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            }]
        }
        Some("assistant") => map_content_blocks(value, false),
        Some("user") => map_content_blocks(value, true),
        Some("result") => map_result(value),
        Some("log") => {
            let log = value.get("log");
            vec![AgentEvent::Log {
                level: log
                    .and_then(|l| l.get("level"))
                    .and_then(Value::as_str)
                    .unwrap_or("info")
                    .to_string(),
                message: log
                    .and_then(|l| l.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            }]
        }
        Some("control_request") => map_control_request(value),
        _ => Vec::new(),
    }
}

/// Map an `assistant` or `user` event's `message.content` blocks.
///
/// `is_user` selects which block shapes are expected (`tool_result` for
/// user messages; `text`/`thinking`/`tool_use` for assistant messages) —
/// unrecognized block types are ignored either way, so passing the wrong
/// flag only means missing events, not a panic.
fn map_content_blocks(value: &Value, is_user: bool) -> Vec<AgentEvent> {
    let Some(blocks) = value
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    let mut events = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") if !is_user => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    events.push(AgentEvent::AssistantText {
                        text: text.to_string(),
                    });
                }
            }
            Some("thinking") if !is_user => {
                if let Some(text) = block.get("thinking").and_then(Value::as_str) {
                    events.push(AgentEvent::Thinking {
                        text: text.to_string(),
                    });
                }
            }
            Some("tool_use") if !is_user => {
                events.push(AgentEvent::ToolCall {
                    id: block.get("id").and_then(Value::as_str).map(str::to_string),
                    name: block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    input: block.get("input").cloned().unwrap_or(Value::Null),
                });
            }
            Some("tool_result") if is_user => {
                events.push(AgentEvent::ToolResult {
                    id: block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    content: block.get("content").cloned().unwrap_or(Value::Null),
                });
            }
            _ => {}
        }
    }
    events
}

/// Map a `result` event: a terminal [`AgentEvent::Result`], plus an
/// [`AgentEvent::Usage`] when either `modelUsage` or `usage` is present.
///
/// Token usage is read per-model from `modelUsage.<model-id>.{input,output}_tokens`
/// first (summed across models, since [`AgentEvent::Usage`] has no per-model
/// breakdown), falling back to the top-level `usage` object — matching
/// `_docs/harness/claude-code.md` §"Output stream protocol"'s note.
fn map_result(value: &Value) -> Vec<AgentEvent> {
    let is_error = value
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let error = if is_error {
        value
            .get("error")
            .and_then(Value::as_str)
            .or_else(|| value.get("result").and_then(Value::as_str))
            .map(str::to_string)
    } else {
        None
    };

    let mut events = vec![AgentEvent::Result {
        success: !is_error,
        error,
    }];

    if let Some(usage) = extract_usage(value) {
        events.push(usage);
    }
    events
}

/// Sum token counts out of `modelUsage` (per-model map), falling back to a
/// single top-level `usage` object. `None` if neither is present.
fn extract_usage(value: &Value) -> Option<AgentEvent> {
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut found = false;

    if let Some(model_usage) = value.get("modelUsage").and_then(Value::as_object) {
        for usage in model_usage.values() {
            if let Some(v) = usage.get("input_tokens").and_then(Value::as_u64) {
                input_tokens += v;
                found = true;
            }
            if let Some(v) = usage.get("output_tokens").and_then(Value::as_u64) {
                output_tokens += v;
                found = true;
            }
        }
    }

    if !found && let Some(usage) = value.get("usage") {
        if let Some(v) = usage.get("input_tokens").and_then(Value::as_u64) {
            input_tokens += v;
            found = true;
        }
        if let Some(v) = usage.get("output_tokens").and_then(Value::as_u64) {
            output_tokens += v;
            found = true;
        }
    }

    found.then_some(AgentEvent::Usage {
        input_tokens: Some(input_tokens),
        output_tokens: Some(output_tokens),
    })
}

/// Map a `control_request` (`_docs/harness/claude-code.md`
/// §"Tool approval in headless mode") to an [`AgentEvent::ApprovalRequest`].
/// Missing `request_id` yields no event (nothing to auto-allow either, in
/// [`read_loop`]).
fn map_control_request(value: &Value) -> Vec<AgentEvent> {
    let Some(request_id) = value.get("request_id").and_then(Value::as_str) else {
        return Vec::new();
    };
    let tool_use = value.get("request").and_then(|r| r.get("tool_use"));
    vec![AgentEvent::ApprovalRequest {
        request_id: request_id.to_string(),
        tool_name: tool_use
            .and_then(|t| t.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        input: tool_use
            .and_then(|t| t.get("input"))
            .cloned()
            .unwrap_or(Value::Null),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_event_system_init_is_session_started() {
        let v: Value =
            serde_json::from_str(r#"{"type":"system","subtype":"init","session_id":"abc"}"#)
                .unwrap();
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::SessionStarted {
                session_id: Some("abc".to_string())
            }]
        );
    }

    #[test]
    fn map_event_assistant_text_thinking_and_tool_use() {
        let v: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"content":[
                {"type":"text","text":"hi there"},
                {"type":"thinking","thinking":"pondering"},
                {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}
            ]}}"#,
        )
        .unwrap();
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![
                AgentEvent::AssistantText {
                    text: "hi there".to_string()
                },
                AgentEvent::Thinking {
                    text: "pondering".to_string()
                },
                AgentEvent::ToolCall {
                    id: Some("t1".to_string()),
                    name: "Bash".to_string(),
                    input: json!({"command": "ls"}),
                },
            ]
        );
    }

    #[test]
    fn map_event_user_tool_result() {
        let v: Value = serde_json::from_str(
            r#"{"type":"user","message":{"content":[
                {"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"ok"}]}
            ]}}"#,
        )
        .unwrap();
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::ToolResult {
                id: Some("t1".to_string()),
                content: json!([{"type": "text", "text": "ok"}]),
            }]
        );
    }

    #[test]
    fn map_event_result_success_with_model_usage() {
        let v: Value = serde_json::from_str(
            r#"{"type":"result","result":"success","is_error":false,"usage":{},
                "modelUsage":{"claude-x":{"input_tokens":10,"output_tokens":20}}}"#,
        )
        .unwrap();
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![
                AgentEvent::Result {
                    success: true,
                    error: None
                },
                AgentEvent::Usage {
                    input_tokens: Some(10),
                    output_tokens: Some(20),
                },
            ]
        );
    }

    #[test]
    fn map_event_result_falls_back_to_top_level_usage() {
        let v: Value = serde_json::from_str(
            r#"{"type":"result","result":"success","is_error":false,
                "usage":{"input_tokens":3,"output_tokens":4}}"#,
        )
        .unwrap();
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![
                AgentEvent::Result {
                    success: true,
                    error: None
                },
                AgentEvent::Usage {
                    input_tokens: Some(3),
                    output_tokens: Some(4),
                },
            ]
        );
    }

    #[test]
    fn map_event_result_error() {
        let v: Value = serde_json::from_str(
            r#"{"type":"result","result":"boom","is_error":true}"#,
        )
        .unwrap();
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::Result {
                success: false,
                error: Some("boom".to_string()),
            }]
        );
    }

    #[test]
    fn map_event_log() {
        let v: Value =
            serde_json::from_str(r#"{"type":"log","log":{"level":"warn","message":"careful"}}"#)
                .unwrap();
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::Log {
                level: "warn".to_string(),
                message: "careful".to_string(),
            }]
        );
    }

    #[test]
    fn map_event_control_request_is_approval_request() {
        let v: Value = serde_json::from_str(
            r#"{"type":"control_request","request_id":"r1",
                "request":{"type":"tool_use","tool_use":{"id":"t1","name":"Bash","input":{"command":"ls"}}}}"#,
        )
        .unwrap();
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::ApprovalRequest {
                request_id: "r1".to_string(),
                tool_name: "Bash".to_string(),
                input: json!({"command": "ls"}),
            }]
        );
    }

    #[test]
    fn map_event_unknown_type_is_ignored() {
        let v: Value = serde_json::from_str(r#"{"type":"something_new","foo":"bar"}"#).unwrap();
        assert_eq!(map_event(&v), Vec::new());
    }

    #[test]
    fn control_response_shape_matches_the_protocol() {
        let v = control_response("r1", "allow", json!({}));
        assert_eq!(
            v,
            json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": "r1",
                    "response": {
                        "behavior": "allow",
                        "updatedInput": {},
                    },
                },
            })
        );
    }
}
