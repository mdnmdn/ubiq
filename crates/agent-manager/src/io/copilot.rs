//! GitHub Copilot's NDJSON bridge — a concrete [`super::IoBridge`] implementation.
//!
//! Speaks the wire protocol documented in `_docs/harness/copilot.md`
//! §"Orchestration / headless invocation" / §"Output stream protocol": one
//! JSON object per line on stdout (events only; Copilot CLI is **one-shot**,
//! with the prompt delivered via `-p` flag at launch, not over stdin). No input
//! stream exists.
//!
//! This is **core** (always compiled, no feature gate): only `std::process`,
//! `std::sync`, `std::thread`, `serde_json` and `tracing` are used, matching
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
//! `AgentEvent::TurnEnded` (if not already sent by an explicit error event)
//! and closes the channel.
//!
//! The same reader-thread architecture as [`super::jsonl`] prevents blocking
//! on writes, even though Copilot takes no stdin input: the child might
//! buffer output, and keeping the reader draining stdout prevents that from
//! stalling the process (a full pipe blocks the producer).
//!
//! ## What Copilot never reports
//!
//! There is no on-stream approval handshake — the CLI runs headless with
//! `--allow-all --no-ask-user` — so [`AgentEvent::PermissionRequest`] never
//! appears. Nor is there a `tool`-start event: `tool.execution_complete` is
//! the only tool event the protocol has, so this bridge only ever emits a
//! [`AgentEvent::ToolCallUpdate`] for a call it never announced, and that
//! update carries no `kind` (nothing here ever learns the tool's name).
//! And no event carries a context window — or any token count at all — so
//! [`AgentEvent::UsageUpdate`] is never emitted either.
//!
//! ## Logging
//!
//! Every raw line is a `trace!`; every mapped event is a `debug!`. Raw frames
//! carry prompts and file contents, which is why they sit a level below
//! everything else.

use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdout};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{
    AgentEvent, AgentInput, Content, IoBridge, StopReason, ToolCallUpdate, ToolContent, ToolStatus,
};

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

        let reader = std::thread::spawn(move || read_loop(stdout, tx));

        Ok(Self {
            child,
            events: rx,
            reader: Some(reader),
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
            AgentInput::AnswerPermission { .. } => {
                // Copilot runs headless with `--allow-all --no-ask-user`, so
                // no `PermissionRequest` is ever emitted for this to answer.
                // A no-op rather than an error: nothing is waiting.
                Ok(())
            }
            AgentInput::Cancel => {
                // Best-effort signal: kill the process so the run stops.
                let _ = self.child.kill();
                Ok(())
            }
            AgentInput::SetConfigOption { config_id, .. } => Err(anyhow::anyhow!(
                "copilot's one-shot bridge cannot change '{config_id}' mid-session"
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

    // `input()` keeps the default `None`: Copilot takes no input after
    // launch (the prompt is argv-only), so there is no sink to hand a caller.
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
/// emit a terminal [`AgentEvent::TurnEnded`] if one hasn't been sent already
/// (no explicit error), then drop `tx` (closing the channel).
fn read_loop(stdout: ChildStdout, tx: mpsc::Sender<AgentEvent>) {
    // Whether a terminal `TurnEnded` has already gone out via the stream (an explicit
    // `result`/`session.error`), so EOF doesn't send a second one.
    let mut turn_ended = false;
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        tracing::trace!(direction = "in", frame = %line, "copilot ndjson");
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            // Not a recognized JSON line — ignore.
            continue;
        };

        for ev in map_event(&value) {
            tracing::debug!(event = ?ev, "copilot event");
            if matches!(ev, AgentEvent::TurnEnded { .. }) {
                turn_ended = true;
            }
            if tx.send(ev).is_err() {
                // No one is listening anymore.
                return;
            }
        }
    }

    // EOF: emit a successful turn end if the stream didn't already end one.
    if !turn_ended {
        let ev = AgentEvent::TurnEnded {
            stop_reason: StopReason::EndTurn,
            error: None,
        };
        tracing::debug!(event = ?ev, "copilot event");
        let _ = tx.send(ev);
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
                model: value
                    .get("data")
                    .and_then(|d| d.get("selectedModel"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                mode: None,
                tools: Vec::new(),
                agents: Vec::new(),
            }]
        }
        Some("assistant.message_delta") => {
            if let Some(text) = value
                .get("data")
                .and_then(|d| d.get("deltaContent"))
                .and_then(Value::as_str)
            {
                vec![AgentEvent::AgentMessageChunk {
                    content: Content::text(text),
                    // Copilot doesn't tag a delta with a message id.
                    message_id: None,
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
                vec![AgentEvent::AgentThoughtChunk {
                    content: Content::text(text),
                    message_id: None,
                }]
            } else {
                Vec::new()
            }
        }
        Some("tool.execution_complete") => {
            // No tool-call/start event is documented for this protocol, only
            // completion, so this is an update to a call this bridge never
            // announced — no `kind`, since the tool's name never appears.
            let data = value.get("data");
            let id = data
                .and_then(|d| d.get("toolCallId"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let succeeded = data
                .and_then(|d| d.get("success"))
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let output = data.and_then(|d| d.get("result")).cloned();

            vec![AgentEvent::ToolCallUpdate {
                update: ToolCallUpdate {
                    content: output.as_ref().and_then(Value::as_str).map(|text| {
                        vec![ToolContent::Content {
                            content: Content::text(text),
                        }]
                    }),
                    raw_output: output,
                    ..ToolCallUpdate::finished(
                        id,
                        if succeeded {
                            ToolStatus::Completed
                        } else {
                            ToolStatus::Failed
                        },
                    )
                },
            }]
        }
        Some("result") => {
            let exit_code = value.get("exitCode").and_then(Value::as_i64);
            let stop_reason = if exit_code == Some(0) {
                StopReason::EndTurn
            } else {
                StopReason::Failed
            };
            vec![AgentEvent::TurnEnded {
                stop_reason,
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
            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::Failed,
                error: Some(message),
            }]
        }
        other => {
            tracing::trace!(kind = ?other, "copilot event ignored");
            Vec::new()
        }
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
                session_id: Some("sess-123".to_string()),
                model: Some("claude".to_string()),
                mode: None,
                tools: Vec::new(),
                agents: Vec::new(),
            }]
        );
    }

    #[test]
    fn map_event_assistant_message_delta_is_agent_message_chunk() {
        let v = json!({"type":"assistant.message_delta","data":{"deltaContent":"hello world"}});
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
    fn map_event_assistant_reasoning_is_agent_thought_chunk() {
        let v =
            json!({"type":"assistant.reasoning","data":{"content":"thinking about the problem"}});
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::AgentThoughtChunk {
                content: Content::text("thinking about the problem"),
                message_id: None,
            }]
        );
    }

    #[test]
    fn map_event_tool_execution_complete_is_a_completed_update() {
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
        let AgentEvent::ToolCallUpdate { update } = &events[0] else {
            panic!("expected an update, got {events:?}");
        };
        assert_eq!(update.id, "tool-1");
        assert_eq!(update.status, Some(ToolStatus::Completed));
        assert_eq!(
            update.kind, None,
            "the tool's name never appears on the wire"
        );
        assert_eq!(
            update.raw_output,
            Some(json!({"stdout":"file.txt\nfile2.txt"}))
        );
    }

    #[test]
    fn map_event_tool_execution_complete_failure_is_a_failed_update() {
        let v = json!({
            "type":"tool.execution_complete",
            "data":{"toolCallId":"tool-1","success":false,"result":"boom"}
        });
        let events = map_event(&v);
        let AgentEvent::ToolCallUpdate { update } = &events[0] else {
            panic!("expected an update, got {events:?}");
        };
        assert_eq!(update.status, Some(ToolStatus::Failed));
        assert_eq!(
            update.content,
            Some(vec![ToolContent::Content {
                content: Content::text("boom"),
            }])
        );
    }

    #[test]
    fn map_event_result_success_ends_the_turn() {
        let v = json!({"type":"result","sessionId":"sess-123","exitCode":0});
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::EndTurn,
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
            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::Failed,
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
            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::Failed,
                error: None,
            }]
        );
    }

    #[test]
    fn map_event_session_error_ends_the_turn_as_failed() {
        let v = json!({"type":"session.error","data":{"message":"something went wrong"}});
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
    fn map_event_session_error_without_message_defaults_to_unknown_error() {
        let v = json!({"type":"session.error","data":{}});
        let events = map_event(&v);
        assert_eq!(
            events,
            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::Failed,
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
