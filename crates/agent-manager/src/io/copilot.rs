//! GitHub Copilot's NDJSON bridge — a concrete [`super::IoBridge`] implementation.
//!
//! Speaks the wire protocol documented in `_docs/harness/copilot.md`
//! §"Orchestration / headless invocation" / §"Output stream protocol": one
//! JSON object per line on stdout (events only; Copilot CLI is **one-shot**,
//! with the prompt delivered via `-p` flag at launch, not over stdin). No input
//! stream exists.
//!
//! This is **core** (always compiled, no feature gate): only `std::process`,
//! `std::sync`, `std::thread`, and `serde_json` are used, matching
//! [`super::structured`]'s "no pty, no clap" discipline so a lib-mode
//! embedder can use it without the `pty`/`cli` features.
//!
//! ## Design
//!
//! [`CopilotBridge::new`] takes ownership of a spawned [`std::process::Child`]
//! (from [`super::spawn_piped`]), splits off its stdout, and spawns a
//! dedicated **reader thread** that scans stdout line-by-line and pushes mapped
//! [`AgentEvent`]s onto an `mpsc` channel. stdin is dropped immediately since
//! Copilot is one-shot and accepts no further input (the prompt is part of
//! the argv). On stdout EOF, the reader thread emits a terminal
//! `AgentEvent::Result` (if not already sent by an explicit error event)
//! and closes the channel.
//!
//! The same reader-thread architecture as [`super::jsonl`] prevents blocking
//! on writes, even though Copilot takes no stdin input: the child might
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
/// `_docs/harness/copilot.md` §"Process lifecycle".
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// A live bridge to a Copilot CLI process running headlessly with
/// `--output-format json` (one-shot NDJSON on stdout, no input stream).
pub struct CopilotBridge {
    child: Child,
    events: mpsc::Receiver<AgentEvent>,
    reader: Option<std::thread::JoinHandle<()>>,
    /// Track whether a terminal `Result` event has been emitted via the
    /// stream (an explicit error) so we don't double-emit at EOF.
    /// Shared with the reader thread via Arc<Mutex<bool>>.
    #[allow(dead_code)]
    result_sent: Arc<Mutex<bool>>,
}

impl CopilotBridge {
    /// Wrap an already-spawned Copilot CLI child (from [`super::spawn_piped`])
    /// as a [`CopilotBridge`].
    ///
    /// Takes ownership of the child and its stdout; stdin is immediately
    /// dropped (Copilot is one-shot). Returns an error if stdout is not
    /// piped (a programmer error — [`super::spawn_piped`] always pipes both).
    pub fn new(mut child: Child) -> crate::Result<Self> {
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("child stdout is not piped"))?;

        // Drop stdin immediately; Copilot takes no input.
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

impl IoBridge for CopilotBridge {
    fn send(&mut self, input: AgentInput) -> crate::Result<()> {
        match input {
            AgentInput::Prompt { .. } => {
                // Copilot is one-shot: the prompt is delivered via `-p` at
                // launch. Sending a prompt on the bridge is a no-op.
                Ok(())
            }
            AgentInput::ApproveTool { .. } => {
                // Copilot runs headless with `--allow-all --no-ask-user`,
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

impl Drop for CopilotBridge {
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
/// `_docs/harness/copilot.md` §"Output stream protocol".
fn map_event(value: &Value) -> Vec<AgentEvent> {
    match value.get("type").and_then(Value::as_str) {
        Some("session.start") => {
            vec![AgentEvent::SessionStarted {
                session_id: value
                    .get("data")
                    .and_then(|d| d.get("sessionId"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
            }]
        }
        Some("assistant.message_delta") => {
            if let Some(text) = value
                .get("data")
                .and_then(|d| d.get("deltaContent"))
                .and_then(Value::as_str)
            {
                vec![AgentEvent::AssistantText {
                    text: text.to_string(),
                }]
            } else {
                Vec::new()
            }
        }
        Some("assistant.reasoning") => {
            if let Some(text) = value
                .get("data")
                .and_then(|d| d.get("content"))
                .and_then(Value::as_str)
            {
                vec![AgentEvent::Thinking {
                    text: text.to_string(),
                }]
            } else {
                Vec::new()
            }
        }
        Some("tool.execution_complete") => {
            // Note: no tool-call/start event is documented for this protocol,
            // only completion. We emit only the result event.
            vec![AgentEvent::ToolResult {
                id: value
                    .get("data")
                    .and_then(|d| d.get("toolCallId"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                content: value
                    .get("data")
                    .and_then(|d| d.get("result"))
                    .cloned()
                    .unwrap_or(Value::Null),
            }]
        }
        Some("result") => {
            let exit_code = value.get("exitCode").and_then(Value::as_i64);
            let success = exit_code == Some(0);
            vec![AgentEvent::Result {
                success,
                error: None,
            }]
        }
        Some("session.error") => {
            let message = value
                .get("data")
                .and_then(|d| d.get("message"))
                .and_then(Value::as_str)
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
    fn map_event_session_start_is_session_started() {
        let v = json!({"type":"session.start","data":{"sessionId":"sess-123","selectedModel":"claude"}});
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::SessionStarted {
                session_id: Some("sess-123".to_string())
            }]
        );
    }

    #[test]
    fn map_event_assistant_message_delta_is_assistant_text() {
        let v = json!({"type":"assistant.message_delta","data":{"deltaContent":"hello world"}});
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::AssistantText {
                text: "hello world".to_string()
            }]
        );
    }

    #[test]
    fn map_event_assistant_reasoning_is_thinking() {
        let v = json!({"type":"assistant.reasoning","data":{"content":"thinking about the problem"}});
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::Thinking {
                text: "thinking about the problem".to_string()
            }]
        );
    }

    #[test]
    fn map_event_tool_execution_complete_is_tool_result() {
        let v = json!({
            "type":"tool.execution_complete",
            "data":{
                "toolCallId":"tool-1",
                "success":true,
                "result":{"stdout":"file.txt\nfile2.txt"},
                "model":"claude"
            }
        });
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::ToolResult {
                id: Some("tool-1".to_string()),
                content: json!({"stdout":"file.txt\nfile2.txt"}),
            }]
        );
    }

    #[test]
    fn map_event_result_success() {
        let v = json!({"type":"result","sessionId":"sess-123","exitCode":0});
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::Result {
                success: true,
                error: None,
            }]
        );
    }

    #[test]
    fn map_event_result_failure_with_nonzero_exit_code() {
        let v = json!({"type":"result","sessionId":"sess-123","exitCode":1});
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::Result {
                success: false,
                error: None,
            }]
        );
    }

    #[test]
    fn map_event_result_failure_missing_exit_code() {
        let v = json!({"type":"result","sessionId":"sess-123"});
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::Result {
                success: false,
                error: None,
            }]
        );
    }

    #[test]
    fn map_event_session_error_is_result_failure() {
        let v = json!({"type":"session.error","data":{"message":"something went wrong"}});
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
    fn map_event_session_error_without_message_defaults_to_unknown_error() {
        let v = json!({"type":"session.error","data":{}});
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::Result {
                success: false,
                error: Some("unknown error".to_string()),
            }]
        );
    }

    #[test]
    fn map_event_unknown_type_is_ignored() {
        let v = json!({"type":"something_new","foo":"bar"});
        assert_eq!(map_event(&v), Vec::new());
    }

    #[test]
    fn map_event_assistant_message_delta_without_content_is_ignored() {
        let v = json!({"type":"assistant.message_delta","data":{}});
        assert_eq!(map_event(&v), Vec::new());
    }

    #[test]
    fn map_event_assistant_reasoning_without_content_is_ignored() {
        let v = json!({"type":"assistant.reasoning","data":{}});
        assert_eq!(map_event(&v), Vec::new());
    }
}
