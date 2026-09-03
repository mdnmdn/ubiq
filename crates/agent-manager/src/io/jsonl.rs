//! Claude Code's `stream-json` (NDJSON) bridge — the first concrete
//! [`super::IoBridge`] implementation.
//!
//! Speaks the wire protocol documented in `_docs/harness/claude-code.md`
//! §"Output stream protocol" / §"Tool approval in headless mode": one JSON
//! object per line on stdout (events), one JSON object per line on stdin
//! (prompts and `control_response` answers).
//!
//! This is **core** (always compiled, no feature gate): only `std::process`,
//! `std::sync`, `std::thread`, `serde_json` and `tracing` are used, matching
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
//! stdin is shared as `Arc<Mutex<Option<ChildStdin>>>` because *three*
//! producers write to it: [`JsonlBridge::send`], the reader thread itself
//! (auto-allow `control_response` lines, written the moment a
//! `control_request` is scanned off stdout, so an unattended run makes
//! progress without a consumer answering), and any [`JsonlInput`] handed out
//! through [`IoBridge::input`]. Wrapping it in `Option` (rather than just
//! `Mutex<ChildStdin>`) gives [`AgentInput::Cancel`] and [`Drop`] a way to
//! *close* stdin while it is shared.
//!
//! ## Mapping is stateful, and has to be
//!
//! [`Mapper`] remembers the context window each model reported, because
//! Claude only states it in a `result` event while per-message `usage`
//! arrives all through a turn. A usage event is emitted only once a window is
//! known for that model — a ratio with an invented denominator is worse than
//! no ratio.
//!
//! ## Logging
//!
//! Every raw line, in either direction, is a `trace!`; every mapped event is
//! a `debug!`. Raw frames carry prompts and file contents, which is why they
//! sit a level below everything else: an embedder's default filter collects
//! `debug` and leaves them out until someone asks for them by name.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{
    AgentEvent, AgentInput, AgentInputSink, Content, IoBridge, PermissionKind, PermissionOption,
    PermissionOutcome, StopReason, ToolCall, ToolCallUpdate, ToolContent, ToolKind, ToolLocation,
    ToolStatus,
};

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

/// The detached input side of a [`JsonlBridge`], for a caller pumping events
/// on one thread and prompting from another. See [`AgentInputSink`].
pub struct JsonlInput {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
}

impl AgentInputSink for JsonlInput {
    fn send(&self, input: AgentInput) -> crate::Result<()> {
        write_input(&self.stdin, input)
    }
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
        write_input(&self.stdin, input)
    }

    fn next_event(&mut self) -> crate::Result<Option<AgentEvent>> {
        match self.events.recv() {
            Ok(ev) => Ok(Some(ev)),
            // Sender dropped == reader thread exited == stdout hit EOF.
            Err(mpsc::RecvError) => Ok(None),
        }
    }

    fn input(&self) -> Option<Arc<dyn AgentInputSink>> {
        Some(Arc::new(JsonlInput {
            stdin: Arc::clone(&self.stdin),
        }))
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

/// Turn one [`AgentInput`] into the line Claude expects, and write it.
///
/// Shared by [`JsonlBridge::send`] and [`JsonlInput`] so the two cannot drift
/// apart — a prompt sent from a pump thread has to reach the child in exactly
/// the same shape as one sent from the owner's.
fn write_input(stdin: &Arc<Mutex<Option<ChildStdin>>>, input: AgentInput) -> crate::Result<()> {
    match input {
        AgentInput::Prompt { ref content } => {
            let blocks: Vec<Value> = content
                .iter()
                .filter_map(Content::as_text)
                .map(|text| json!({"type": "text", "text": text}))
                .collect();
            let line = json!({
                "type": "user",
                "message": { "role": "user", "content": blocks },
            });
            write_line(stdin, &line)
        }
        AgentInput::AnswerPermission {
            request_id,
            outcome,
            updated_input,
        } => {
            let behavior = match &outcome {
                PermissionOutcome::Selected { option_id } => option_id.as_str(),
                // A cancelled turn denies whatever was waiting on a human.
                PermissionOutcome::Cancelled => "deny",
            };
            let line = control_response(
                &request_id,
                behavior,
                updated_input.unwrap_or_else(|| json!({})),
            );
            write_line(stdin, &line)
        }
        AgentInput::Cancel => {
            // Close stdin so Claude Code sees EOF and stops; the reader
            // thread keeps draining stdout until the process actually
            // exits (see `_docs/harness/claude-code.md`
            // §"Process lifecycle").
            if let Ok(mut guard) = stdin.lock() {
                *guard = None;
            }
            Ok(())
        }
        AgentInput::SetConfigOption { config_id, .. } => Err(anyhow::anyhow!(
            "claude-code's stream-json bridge cannot change '{config_id}' mid-session"
        )),
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
    let mut mapper = Mapper::default();
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        tracing::trace!(direction = "in", frame = %line, "claude stream-json");
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            // Not a recognized JSON line — ignore rather than error, per
            // the mapping contract ("don't error on unrecognized lines").
            continue;
        };

        let request_id = is_control_request(&value)
            .then(|| value.get("request_id").and_then(Value::as_str))
            .flatten()
            .map(str::to_string);

        for ev in mapper.map_event(&value) {
            tracing::debug!(event = ?ev, "claude event");
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
/// [`AgentInput::Cancel`]) is a silent no-op rather than an error — the
/// process is already being told to stop.
fn write_line(stdin: &Arc<Mutex<Option<ChildStdin>>>, value: &Value) -> crate::Result<()> {
    let mut guard = stdin
        .lock()
        .map_err(|_| anyhow::anyhow!("jsonl bridge stdin lock poisoned"))?;
    if let Some(stdin) = guard.as_mut() {
        tracing::trace!(direction = "out", frame = %value, "claude stream-json");
        writeln!(stdin, "{value}")?;
        stdin.flush()?;
    }
    Ok(())
}

// ── mapping ────────────────────────────────────────────────────────────

/// What the mapper has to remember across lines.
///
/// Only one thing so far, and it is unavoidable: a model's context window is
/// stated in `result.modelUsage` and nowhere else, while per-message `usage`
/// arrives throughout a turn. Without the window a usage event has no
/// denominator, so the first turn reports its context at the end and every
/// turn after it reports as it goes.
#[derive(Default)]
struct Mapper {
    windows: HashMap<String, u64>,
}

impl Mapper {
    /// Map one parsed NDJSON stdout line to zero or more [`AgentEvent`]s.
    ///
    /// See the mapping table in `_docs/harness/claude-code.md`
    /// §"Output stream protocol".
    fn map_event(&mut self, value: &Value) -> Vec<AgentEvent> {
        match value.get("type").and_then(Value::as_str) {
            Some("system") if value.get("subtype").and_then(Value::as_str) == Some("init") => {
                vec![map_init(value)]
            }
            Some("assistant") => self.map_assistant(value),
            Some("user") => map_user(value),
            Some("result") => self.map_result(value),
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
            other => {
                tracing::trace!(kind = ?other, "claude event ignored");
                Vec::new()
            }
        }
    }

    /// An `assistant` event: its content blocks, then its own token usage if
    /// a window for that model is already known.
    fn map_assistant(&mut self, value: &Value) -> Vec<AgentEvent> {
        let mut events = map_content_blocks(value, false);
        if let Some(usage) = self.message_usage(value) {
            events.push(usage);
        }
        events
    }

    /// Per-message accounting. Claude reports the tokens *this* message cost;
    /// what fills a context window is the input plus everything read from or
    /// written to the cache.
    fn message_usage(&self, value: &Value) -> Option<AgentEvent> {
        let message = value.get("message")?;
        let usage = message.get("usage")?;
        let model = message.get("model").and_then(Value::as_str)?;
        let size = *self.windows.get(model)?;

        let field = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
        let used = field("input_tokens")
            + field("cache_read_input_tokens")
            + field("cache_creation_input_tokens");

        Some(AgentEvent::UsageUpdate {
            used,
            size,
            cost: None,
            model: Some(model.to_string()),
        })
    }

    /// A `result` event: the turn's usage — which is where a context window
    /// is learned — and then the turn's end.
    fn map_result(&mut self, value: &Value) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        if let Some(usage) = self.turn_usage(value) {
            events.push(usage);
        }

        let is_error = value
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let subtype = value.get("subtype").and_then(Value::as_str);
        let stop_reason = match (is_error, subtype) {
            (_, Some("error_max_turns")) => StopReason::MaxTurnRequests,
            (true, _) => StopReason::Failed,
            _ => StopReason::EndTurn,
        };
        let error = is_error
            .then(|| {
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .or_else(|| value.get("result").and_then(Value::as_str))
            })
            .flatten()
            .map(str::to_string);

        events.push(AgentEvent::TurnEnded { stop_reason, error });
        events
    }

    /// Read `modelUsage` — **camelCase on the wire**, whatever an older
    /// version of the harness contract said — remember each model's context
    /// window, and report the turn's own totals.
    ///
    /// Falls back to the top-level `usage` object, which *is* snake_case, for
    /// a run that reported no per-model breakdown; that path has no window,
    /// so it can only contribute a cost.
    fn turn_usage(&mut self, value: &Value) -> Option<AgentEvent> {
        let cost = value
            .get("total_cost_usd")
            .and_then(Value::as_f64)
            .map(|amount| super::Cost {
                amount,
                currency: "USD".to_string(),
            });

        let model_usage = value.get("modelUsage").and_then(Value::as_object);
        let Some(model_usage) = model_usage else {
            return cost.map(|cost| AgentEvent::UsageUpdate {
                used: 0,
                size: 0,
                cost: Some(cost),
                model: None,
            });
        };

        // Report the model that did the most work — a turn that fell back to
        // a small model for one call should still show the main model's ring.
        let mut best: Option<(String, u64, u64)> = None;
        for (model, usage) in model_usage {
            let field = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
            let window = field("contextWindow");
            if window > 0 {
                self.windows.insert(model.clone(), window);
            }
            let used = field("inputTokens")
                + field("cacheReadInputTokens")
                + field("cacheCreationInputTokens");
            if best.as_ref().is_none_or(|(_, b, _)| used > *b) {
                best = Some((model.clone(), used, window));
            }
        }

        let (model, used, size) = best?;
        Some(AgentEvent::UsageUpdate {
            used,
            size,
            cost,
            model: Some(model),
        })
    }
}

/// A `system`/`init` event: the session's id and everything it can do.
fn map_init(value: &Value) -> AgentEvent {
    let strings = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        item.as_str().map(str::to_string).or_else(|| {
                            item.get("name").and_then(Value::as_str).map(str::to_string)
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    AgentEvent::SessionStarted {
        session_id: value
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        model: value
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        mode: value
            .get("permissionMode")
            .and_then(Value::as_str)
            .map(str::to_string),
        tools: strings("tools"),
        agents: strings("agents"),
    }
}

/// A `user` event: what the harness received from us, and any tool results
/// carried back on the same turn.
fn map_user(value: &Value) -> Vec<AgentEvent> {
    map_content_blocks(value, true)
}

/// Map an `assistant` or `user` event's `message.content` blocks.
///
/// `is_user` selects which block shapes are expected (`tool_result` and the
/// user's own text for user messages; `text`/`thinking`/`tool_use` for
/// assistant messages) — unrecognized block types are ignored either way, so
/// passing the wrong flag only means missing events, not a panic.
fn map_content_blocks(value: &Value, is_user: bool) -> Vec<AgentEvent> {
    let message = value.get("message");
    let message_id = message
        .and_then(|m| m.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let Some(blocks) = message
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    let mut events = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    let content = Content::text(text);
                    let message_id = message_id.clone();
                    events.push(if is_user {
                        AgentEvent::UserMessageChunk {
                            content,
                            message_id,
                        }
                    } else {
                        AgentEvent::AgentMessageChunk {
                            content,
                            message_id,
                        }
                    });
                }
            }
            Some("thinking") if !is_user => {
                if let Some(text) = block.get("thinking").and_then(Value::as_str) {
                    events.push(AgentEvent::AgentThoughtChunk {
                        content: Content::text(text),
                        message_id: message_id.clone(),
                    });
                }
            }
            Some("tool_use") if !is_user => {
                events.push(AgentEvent::ToolCall {
                    call: map_tool_use(block),
                });
            }
            Some("tool_result") if is_user => {
                events.push(AgentEvent::ToolCallUpdate {
                    update: map_tool_result(block),
                });
            }
            _ => {}
        }
    }
    events
}

/// A `tool_use` block, in the shape a transcript draws: a verb, a target, and
/// — for an edit — the diff itself, which Claude puts right in the input.
fn map_tool_use(block: &Value) -> ToolCall {
    let name = block
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let input = block.get("input");
    let string = |key: &str| input.and_then(|i| i.get(key)).and_then(Value::as_str);

    let kind = tool_kind(name);
    let path = string("file_path").or_else(|| string("path"));
    let target = path
        .or_else(|| string("command"))
        .or_else(|| string("pattern"))
        .or_else(|| string("url"));

    let title = match target {
        Some(target) => format!("{name} {target}"),
        None => name.to_string(),
    };

    let mut call = ToolCall::new(
        block.get("id").and_then(Value::as_str).unwrap_or(name),
        title,
    );
    call.kind = kind;
    call.status = ToolStatus::InProgress;
    call.raw_input = input.cloned();
    if let Some(path) = path {
        call.locations = vec![ToolLocation {
            path: path.to_string(),
            line: None,
        }];
        if let Some(diff) = edit_diff(name, path, input) {
            call.content = vec![diff];
        }
    }
    call
}

/// The diff an `Edit` or a `Write` already carries in its input. Anything
/// else has no diff to show, and inventing one from tool output would be
/// guessing.
fn edit_diff(name: &str, path: &str, input: Option<&Value>) -> Option<ToolContent> {
    let input = input?;
    let string = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_string);
    match name {
        "Edit" => Some(ToolContent::Diff {
            path: path.to_string(),
            old_text: string("old_string"),
            new_text: string("new_string")?,
        }),
        "Write" => Some(ToolContent::Diff {
            path: path.to_string(),
            old_text: None,
            new_text: string("content")?,
        }),
        _ => None,
    }
}

/// Claude's tool names onto the ten kinds a consumer draws. An unknown name
/// is [`ToolKind::Other`] rather than a guess.
fn tool_kind(name: &str) -> ToolKind {
    match name {
        "Read" | "NotebookRead" | "Glob" | "LS" => ToolKind::Read,
        "Edit" | "Write" | "NotebookEdit" | "MultiEdit" => ToolKind::Edit,
        "Bash" | "BashOutput" | "KillShell" => ToolKind::Execute,
        "Grep" => ToolKind::Search,
        "WebFetch" => ToolKind::Fetch,
        "WebSearch" => ToolKind::Search,
        "Task" | "TodoWrite" | "ExitPlanMode" => ToolKind::Think,
        _ => ToolKind::Other,
    }
}

/// A `tool_result` block: the call named by `tool_use_id` has finished.
///
/// A `status` of `async_launched` is the one progress signal between a tool
/// starting and finishing, so it stays in progress rather than completing.
fn map_tool_result(block: &Value) -> ToolCallUpdate {
    let failed = block
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let launched = block.get("status").and_then(Value::as_str) == Some("async_launched");

    let status = match (failed, launched) {
        (true, _) => ToolStatus::Failed,
        (_, true) => ToolStatus::InProgress,
        _ => ToolStatus::Completed,
    };

    let content = block.get("content").and_then(result_text).map(|text| {
        vec![ToolContent::Content {
            content: Content::text(text),
        }]
    });

    ToolCallUpdate {
        id: block
            .get("tool_use_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        status: Some(status),
        content,
        raw_output: block.get("content").cloned(),
        ..ToolCallUpdate::default()
    }
}

/// A tool result's content is either a bare string or a list of blocks. Take
/// whatever prose is in it and leave the rest to `raw_output`.
fn result_text(content: &Value) -> Option<String> {
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(blocks) => {
            let text: Vec<&str> = blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect();
            (!text.is_empty()).then(|| text.join("\n"))
        }
        _ => None,
    }
}

/// Map a `control_request` (`_docs/harness/claude-code.md`
/// §"Tool approval in headless mode") to an [`AgentEvent::PermissionRequest`],
/// carrying the whole tool call so a dialog can show what it is authorising.
///
/// Missing `request_id` yields no event (nothing to auto-allow either, in
/// [`read_loop`]).
///
/// The options are the four ACP kinds, and their ids are the `behavior`
/// strings Claude's `control_response` expects — so answering is a
/// pass-through rather than a second mapping. "Always" is offered because the
/// vocabulary has it; remembering it is the caller's job, and nothing does
/// yet.
fn map_control_request(value: &Value) -> Vec<AgentEvent> {
    let Some(request_id) = value.get("request_id").and_then(Value::as_str) else {
        return Vec::new();
    };
    let tool_use = value.get("request").and_then(|r| r.get("tool_use"));
    let call = tool_use.map(map_tool_use).unwrap_or_else(|| {
        let mut call = ToolCall::new(request_id, "tool");
        call.status = ToolStatus::Pending;
        call
    });

    let options = vec![
        PermissionOption {
            option_id: "allow".to_string(),
            name: "Allow".to_string(),
            kind: PermissionKind::AllowOnce,
        },
        PermissionOption {
            option_id: "deny".to_string(),
            name: "Deny".to_string(),
            kind: PermissionKind::RejectOnce,
        },
    ];

    vec![AgentEvent::PermissionRequest {
        request_id: request_id.to_string(),
        tool_call: ToolCallUpdate {
            id: call.id,
            title: Some(call.title),
            kind: Some(call.kind),
            status: Some(ToolStatus::Pending),
            content: (!call.content.is_empty()).then_some(call.content),
            locations: (!call.locations.is_empty()).then_some(call.locations),
            raw_output: None,
        },
        options,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(json: &str) -> Vec<AgentEvent> {
        let value: Value = serde_json::from_str(json).unwrap();
        Mapper::default().map_event(&value)
    }

    #[test]
    fn system_init_carries_the_session_and_what_it_can_do() {
        let events = map(
            r#"{"type":"system","subtype":"init","session_id":"abc","model":"claude-opus-5",
                "permissionMode":"bypassPermissions","tools":["Read","Bash"],
                "agents":["Explore"]}"#,
        );
        assert_eq!(
            events,
            vec![AgentEvent::SessionStarted {
                session_id: Some("abc".to_string()),
                model: Some("claude-opus-5".to_string()),
                mode: Some("bypassPermissions".to_string()),
                tools: vec!["Read".to_string(), "Bash".to_string()],
                agents: vec!["Explore".to_string()],
            }]
        );
    }

    #[test]
    fn assistant_text_thinking_and_tool_use_map_to_chunks_and_a_call() {
        let events = map(r#"{"type":"assistant","message":{"id":"m1","content":[
                {"type":"text","text":"hi there"},
                {"type":"thinking","thinking":"pondering"},
                {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}
            ]}}"#);
        assert_eq!(events.len(), 3);
        assert_eq!(
            events[0],
            AgentEvent::AgentMessageChunk {
                content: Content::text("hi there"),
                message_id: Some("m1".to_string()),
            }
        );
        assert_eq!(
            events[1],
            AgentEvent::AgentThoughtChunk {
                content: Content::text("pondering"),
                message_id: Some("m1".to_string()),
            }
        );
        let AgentEvent::ToolCall { call } = &events[2] else {
            panic!("expected a tool call, got {:?}", events[2]);
        };
        assert_eq!(call.id, "t1");
        assert_eq!(call.title, "Bash ls");
        assert_eq!(call.kind, ToolKind::Execute);
        assert_eq!(call.status, ToolStatus::InProgress);
    }

    /// An `Edit` already carries its own before and after, so the transcript
    /// can draw the diff without asking anyone.
    #[test]
    fn an_edit_carries_its_diff() {
        let events = map(r#"{"type":"assistant","message":{"id":"m1","content":[
                {"type":"tool_use","id":"t1","name":"Edit","input":
                  {"file_path":"/tmp/a.rs","old_string":"one","new_string":"two"}}
            ]}}"#);
        let AgentEvent::ToolCall { call } = &events[0] else {
            panic!("expected a tool call");
        };
        assert_eq!(call.kind, ToolKind::Edit);
        assert_eq!(
            call.content,
            vec![ToolContent::Diff {
                path: "/tmp/a.rs".to_string(),
                old_text: Some("one".to_string()),
                new_text: "two".to_string(),
            }]
        );
        assert_eq!(
            call.locations,
            vec![ToolLocation {
                path: "/tmp/a.rs".to_string(),
                line: None,
            }]
        );
    }

    #[test]
    fn a_tool_result_completes_the_call_it_names() {
        let events = map(r#"{"type":"user","message":{"content":[
                {"type":"tool_result","tool_use_id":"t1","content":"ok"}
            ]}}"#);
        let AgentEvent::ToolCallUpdate { update } = &events[0] else {
            panic!("expected an update, got {events:?}");
        };
        assert_eq!(update.id, "t1");
        assert_eq!(update.status, Some(ToolStatus::Completed));
        assert_eq!(update.title, None, "an update changes only what it names");
    }

    /// The one progress signal between a tool starting and finishing.
    #[test]
    fn an_async_launched_tool_stays_in_progress() {
        let events = map(r#"{"type":"user","message":{"content":[
                {"type":"tool_result","tool_use_id":"t1","status":"async_launched","content":"…"}
            ]}}"#);
        let AgentEvent::ToolCallUpdate { update } = &events[0] else {
            panic!("expected an update");
        };
        assert_eq!(update.status, Some(ToolStatus::InProgress));
    }

    #[test]
    fn a_user_text_block_is_echoed_as_the_users_own_chunk() {
        let events =
            map(r#"{"type":"user","message":{"content":[{"type":"text","text":"do it"}]}}"#);
        assert_eq!(
            events,
            vec![AgentEvent::UserMessageChunk {
                content: Content::text("do it"),
                message_id: None,
            }]
        );
    }

    /// `modelUsage` is camelCase on the wire. The harness contract said
    /// otherwise for a while, and the per-model branch matched nothing.
    #[test]
    fn result_reads_camel_case_model_usage_and_learns_the_window() {
        let value: Value = serde_json::from_str(
            r#"{"type":"result","subtype":"success","total_cost_usd":0.5,"modelUsage":{
                "claude-opus-5":{"inputTokens":100,"outputTokens":20,
                  "cacheReadInputTokens":900,"cacheCreationInputTokens":0,
                  "contextWindow":1000000}}}"#,
        )
        .unwrap();
        let mut mapper = Mapper::default();
        let events = mapper.map_event(&value);

        assert_eq!(
            events[0],
            AgentEvent::UsageUpdate {
                used: 1000,
                size: 1_000_000,
                cost: Some(super::super::Cost {
                    amount: 0.5,
                    currency: "USD".to_string(),
                }),
                model: Some("claude-opus-5".to_string()),
            }
        );
        assert_eq!(
            events[1],
            AgentEvent::TurnEnded {
                stop_reason: StopReason::EndTurn,
                error: None,
            }
        );
        assert_eq!(mapper.windows.get("claude-opus-5"), Some(&1_000_000));
    }

    /// A ratio with an invented denominator is worse than no ratio, so the
    /// first turn reports its context at the end and later ones as they go.
    #[test]
    fn per_message_usage_waits_until_a_window_is_known() {
        let mut mapper = Mapper::default();
        let assistant: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5",
                "usage":{"input_tokens":10,"cache_read_input_tokens":90},
                "content":[{"type":"text","text":"hi"}]}}"#,
        )
        .unwrap();

        let before = mapper.map_event(&assistant);
        assert_eq!(before.len(), 1, "no usage yet: {before:?}");

        mapper.windows.insert("claude-opus-5".to_string(), 200_000);
        let after = mapper.map_event(&assistant);
        assert_eq!(
            after[1],
            AgentEvent::UsageUpdate {
                used: 100,
                size: 200_000,
                cost: None,
                model: Some("claude-opus-5".to_string()),
            }
        );
    }

    #[test]
    fn a_failed_result_ends_the_turn_with_its_reason() {
        let events = map(r#"{"type":"result","is_error":true,"result":"boom"}"#);
        assert_eq!(
            events,
            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::Failed,
                error: Some("boom".to_string()),
            }]
        );
    }

    #[test]
    fn max_turns_is_its_own_stop_reason() {
        let events = map(r#"{"type":"result","subtype":"error_max_turns","is_error":true}"#);
        assert_eq!(
            events,
            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::MaxTurnRequests,
                error: None,
            }]
        );
    }

    #[test]
    fn a_control_request_carries_the_call_it_wants_authorised() {
        let events = map(
            r#"{"type":"control_request","request_id":"r1","request":{"tool_use":
                {"id":"t1","name":"Write","input":{"file_path":"/tmp/a","content":"x"}}}}"#,
        );
        let AgentEvent::PermissionRequest {
            request_id,
            tool_call,
            options,
        } = &events[0]
        else {
            panic!("expected a permission request, got {events:?}");
        };
        assert_eq!(request_id, "r1");
        assert_eq!(tool_call.id, "t1");
        assert_eq!(tool_call.kind, Some(ToolKind::Edit));
        assert_eq!(tool_call.status, Some(ToolStatus::Pending));
        // The ids are Claude's own `behavior` strings, so answering is a
        // pass-through rather than a second mapping.
        assert_eq!(options[0].option_id, "allow");
        assert_eq!(options[1].option_id, "deny");
    }

    #[test]
    fn an_unknown_event_is_dropped_rather_than_failing() {
        assert!(map(r#"{"type":"rate_limit_event","rate_limit_info":{}}"#).is_empty());
    }
}
