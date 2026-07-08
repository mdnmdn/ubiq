//! opencode's NDJSON bridge — a concrete [`super::IoBridge`] implementation.
//!
//! Speaks the wire protocol documented in `_docs/harness/opencode.md`
//! §"Orchestration / headless invocation" / §"Output stream protocol": one
//! JSON object per line on stdout (events only; opencode is **one-shot**, with
//! the prompt delivered via argv at launch, not over stdin). No input stream
//! exists.
//!
//! This is **core** (always compiled, no feature gate): only `std::process`,
//! `std::sync`, `std::thread`, and `serde_json` are used, matching
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
//! `AgentEvent::Result` (if not already sent by an explicit `error` event)
//! and closes the channel.
//!
//! The same reader-thread architecture as [`super::jsonl`] prevents blocking
//! on writes, even though opencode takes no stdin input: the child might
//! buffer output, and keeping the reader draining stdout prevents that from
//! stalling the process (a full pipe blocks the producer).

use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdout};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{AgentEvent, AgentInput, IoBridge};

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
    /// Track whether a terminal `Result` event has been emitted via the
    /// stream (an explicit error) so we don't double-emit at EOF.
    /// Shared with the reader thread via Arc<Mutex<bool>>.
    #[allow(dead_code)]
    result_sent: Arc<Mutex<bool>>,
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
        let result_sent = Arc::new(Mutex::new(false));
        let result_sent_clone = Arc::clone(&result_sent);

        let reader = std::thread::spawn(move || read_loop(stdout, tx, result_sent_clone));

        Ok(Self {
            child,
            events: rx,
            reader: Some(reader),
            result_sent,
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
            AgentInput::ApproveTool { .. } => {
                // opencode runs headless with `--dangerously-skip-permissions`,
                // so every tool runs automatically without an approval handshake.
                // Approval inputs are a no-op.
                Ok(())
            }
            AgentInput::Interrupt => {
                // Best-effort signal: kill the process group so the run stops.
                let _ = self.child.kill();
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
/// emit a terminal `AgentEvent::Result` if one hasn't been sent already
/// (no explicit error), then drop `tx` (closing the channel).
fn read_loop(
    stdout: ChildStdout,
    tx: mpsc::Sender<AgentEvent>,
    result_sent: Arc<Mutex<bool>>,
) {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            // Not a recognized JSON line — ignore.
            continue;
        };

        for ev in map_event(&value) {
            // Track if this is a terminal Result event (from an explicit error
            // in the stream, not EOF).
            if matches!(ev, AgentEvent::Result { .. })
                && let Ok(mut sent) = result_sent.lock()
            {
                *sent = true;
            }
            if tx.send(ev).is_err() {
                // No one is listening anymore.
                return;
            }
        }
    }

    // EOF: emit a success Result if not already sent.
    if let Ok(mut sent) = result_sent.lock() && !*sent {
        let _ = tx.send(AgentEvent::Result {
            success: true,
            error: None,
        });
        *sent = true;
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
            }]
        }
        Some("text") => {
            if let Some(text) = value
                .get("part")
                .and_then(|p| p.get("text"))
                .and_then(Value::as_str)
            {
                vec![AgentEvent::AssistantText {
                    text: text.to_string(),
                }]
            } else {
                Vec::new()
            }
        }
        Some("tool_use") => {
            let part = match value.get("part") {
                Some(p) => p,
                None => return Vec::new(),
            };

            let tool_name = part
                .get("tool")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let call_id = part.get("callID").and_then(Value::as_str).map(str::to_string);

            let state = match part.get("state") {
                Some(s) => s,
                None => return Vec::new(),
            };

            let input = state.get("input").cloned().unwrap_or(Value::Null);
            let status = state.get("status").and_then(Value::as_str).unwrap_or("");
            let output = state.get("output").cloned().unwrap_or(Value::Null);

            // Emit ToolCall with input.
            let mut events = vec![AgentEvent::ToolCall {
                id: call_id.clone(),
                name: tool_name,
                input,
            }];

            // If status is "complete", also emit ToolResult with output.
            if status == "complete" {
                events.push(AgentEvent::ToolResult {
                    id: call_id,
                    content: output,
                });
            }

            events
        }
        Some("step_finish") => {
            if let Some(tokens) = value
                .get("part")
                .and_then(|p| p.get("tokens"))
                .and_then(Value::as_object)
            {
                let input_tokens = tokens
                    .get("input")
                    .and_then(Value::as_u64);
                let output_tokens = tokens
                    .get("output")
                    .and_then(Value::as_u64);

                if input_tokens.is_some() || output_tokens.is_some() {
                    vec![AgentEvent::Usage {
                        input_tokens,
                        output_tokens,
                    }]
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
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

            vec![AgentEvent::Result {
                success: false,
                error: Some(message),
            }]
        }
        _ => Vec::new(),
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
                session_id: Some("sess-123".to_string())
            }]
        );
    }

    #[test]
    fn map_event_text_is_assistant_text() {
        let v = json!({"type":"text","part":{"text":"hello world"}});
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::AssistantText {
                text: "hello world".to_string()
            }]
        );
    }

    #[test]
    fn map_event_tool_use_complete_emits_call_and_result() {
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
        assert_eq!(
            events[0],
            AgentEvent::ToolCall {
                id: Some("call-1".to_string()),
                name: "bash".to_string(),
                input: json!({"command":"ls"}),
            }
        );
        assert_eq!(
            events[1],
            AgentEvent::ToolResult {
                id: Some("call-1".to_string()),
                content: json!("file1.txt\nfile2.txt"),
            }
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
        assert_eq!(
            events[0],
            AgentEvent::ToolCall {
                id: Some("call-1".to_string()),
                name: "bash".to_string(),
                input: json!({"command":"sleep 10"}),
            }
        );
    }

    #[test]
    fn map_event_step_finish_emits_usage() {
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
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::Usage {
                input_tokens: Some(150),
                output_tokens: Some(200),
            }]
        );
    }

    #[test]
    fn map_event_error_is_result_failure() {
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
            vec![AgentEvent::Result {
                success: false,
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
