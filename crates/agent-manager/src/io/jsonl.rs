//! Claude Code's `stream-json` (NDJSON) bridge — the first concrete
//! [`super::IoBridge`] implementation.
//!
//! Speaks the wire protocol documented in `_docs/harness/claude-code.md`
//! §"Output stream protocol" / §"Tool approval in headless mode": one JSON
//! object per line on stdout (events), one JSON object per line on stdin
//! (prompts and `control_response` answers).
//!
//! **This bridge is not superseded by ACP and is not to be deleted** — `D95`. The `claude-code-acp`
//! harness is a sibling that speaks [`super::AcpBridge`]; this is the path with no adapter process
//! and no npm dependency between Ubiq and the model, and it states three things no ACP adapter
//! does: the per-model context window (`result.modelUsage[model].contextWindow`, reconciled through
//! `canonical`), a delegate's spend subtracted out of the turn's so a subagent never moves the
//! parent's ring (`subagent_spend`), and the full five-field [`super::Spend`] breakdown alongside
//! rate-limit windows. Removing it would cost all three.
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
//! stdin is shared as `Arc<Mutex<Option<ChildStdin>>>` because two producers
//! write to it: [`JsonlBridge::send`] and any [`JsonlInput`] handed out
//! through [`IoBridge::input`]. Wrapping it in `Option` (rather than just
//! `Mutex<ChildStdin>`) gives [`AgentInput::Shutdown`] and [`Drop`] a way to
//! *close* stdin while it is shared.
//!
//! ## Cancelling a turn is not ending the session
//!
//! [`AgentInput::Cancel`] writes an `interrupt` `control_request` and leaves stdin open, so the
//! turn aborts and the conversation takes the next prompt on the same process. Closing stdin is
//! [`AgentInput::Shutdown`]'s job, and [`Drop`]'s — a cancel that closed it would end the harness
//! for good, which is not what a stop button means.
//!
//! ## A permission ask is the caller's to answer
//!
//! The reader thread answers no *permission* ask. A `can_use_tool` `control_request` becomes an
//! [`AgentEvent::PermissionRequest`] and stays **outstanding** — recorded in
//! [`Pending`] with the `permission_suggestions` Claude offered — until the
//! caller sends [`AgentInput::AnswerPermission`]. Nothing else can decide:
//! an unanswered request stalls the turn indefinitely, and closing stdin
//! first makes Claude fail the tool with "Tool permission stream closed
//! before response received", so [`AgentInput::Cancel`] denies every
//! outstanding request *before* it interrupts the turn.
//!
//! Any *other* `control_request` subtype is one this bridge does not implement, and no caller will
//! ever answer it, so the reader answers it an error itself rather than letting it stall the turn.
//!
//! ## Mapping is stateful, and has to be
//!
//! [`Mapper`] remembers the context window each model reported, the session's cumulative spend and
//! cost, and which session it has already announced — Claude states windows only in a `result`,
//! states every token and dollar figure cumulatively, and re-announces a session it already
//! announced. What each of `used`, `size`, `spend` and `cost` must mean is the contract on
//! [`AgentEvent::UsageUpdate`]; this bridge fills all four, and emits a usage event only once a
//! window is known for that model — a ratio with an invented denominator is worse than no ratio.
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
    AgentEvent, AgentInput, AgentInputSink, Content, IoBridge, Origin, PermissionKind,
    PermissionOption, PermissionOutcome, RateLimitWindow, Spend, StopReason, ToolCall,
    ToolCallUpdate, ToolContent, ToolKind, ToolLocation, ToolStatus,
};

/// How long [`Drop`] waits for the child to exit after closing stdin before
/// giving up and killing it. Mirrors `_docs/harness/claude-code.md`
/// §"Process lifecycle": "allow ~10s for the process to drain before
/// killing".
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// The permission asks Claude is still waiting on, by `request_id`, each with the
/// `permission_suggestions` it offered (the `{"type":"setMode",…}` objects behind the
/// "always" options in [`map_control_request`]).
///
/// Shared between the reader thread (which records an ask) and every input sink (which answers
/// one, or denies all of them on a cancel).
type Pending = Arc<Mutex<HashMap<String, Vec<Value>>>>;

/// One event, and the stdout line that produced it where there was one.
///
/// A line maps to several events, so the line is shared rather than copied — each event carries a
/// handle onto the same text. A prompt this bridge synthesizes locally has no line at all.
type Framed = (AgentEvent, Option<Arc<str>>);

/// A live bridge to a Claude Code process speaking `stream-json` on
/// stdin/stdout (`-p --output-format stream-json --input-format
/// stream-json`).
pub struct JsonlBridge {
    child: Child,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    /// `None` is the reader thread's own explicit "done" — sent once, right
    /// before it exits, so end-of-stream does not depend on every `Sender`
    /// clone dropping. It cannot: `write_input` and every [`JsonlInput`]
    /// clone hold one for as long as the bridge itself lives, so a plain
    /// `AgentEvent` channel would never see `RecvError` once the process
    /// actually exited.
    events: mpsc::Receiver<Option<Framed>>,
    /// A second handle onto the reader thread's channel, so [`write_input`]
    /// can push a locally-synthesized event — see its doc comment.
    tx: mpsc::Sender<Option<Framed>>,
    /// Permission asks emitted and not yet answered — see [`Pending`].
    pending: Pending,
    reader: Option<std::thread::JoinHandle<()>>,
}

/// The detached input side of a [`JsonlBridge`], for a caller pumping events
/// on one thread and prompting from another. See [`AgentInputSink`].
pub struct JsonlInput {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    tx: mpsc::Sender<Option<Framed>>,
    pending: Pending,
}

impl AgentInputSink for JsonlInput {
    fn send(&self, input: AgentInput) -> crate::Result<()> {
        write_input(&self.stdin, &self.tx, &self.pending, input)
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

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));

        let reader_tx = tx.clone();
        let reader_pending = Arc::clone(&pending);
        let reader_stdin = Arc::clone(&stdin);
        let reader =
            std::thread::spawn(move || read_loop(stdout, reader_pending, reader_stdin, reader_tx));

        Ok(Self {
            child,
            stdin,
            events: rx,
            tx,
            pending,
            reader: Some(reader),
        })
    }
}

impl IoBridge for JsonlBridge {
    fn send(&mut self, input: AgentInput) -> crate::Result<()> {
        write_input(&self.stdin, &self.tx, &self.pending, input)
    }

    fn next_event(&mut self) -> crate::Result<Option<AgentEvent>> {
        Ok(self.next_event_raw()?.map(|(event, _)| event))
    }

    fn next_event_raw(&mut self) -> crate::Result<Option<(AgentEvent, Option<String>)>> {
        match self.events.recv() {
            Ok(Some((ev, raw))) => Ok(Some((ev, raw.map(|line| line.to_string())))),
            // The reader thread's explicit "done".
            Ok(None) => Ok(None),
            // Every sender clone dropped without one ever sending `None` —
            // should not happen in practice, but as honest an EOF as the
            // explicit signal.
            Err(mpsc::RecvError) => Ok(None),
        }
    }

    fn input(&self) -> Option<Arc<dyn AgentInputSink>> {
        Some(Arc::new(JsonlInput {
            stdin: Arc::clone(&self.stdin),
            tx: self.tx.clone(),
            pending: Arc::clone(&self.pending),
        }))
    }

    /// Kill-by-pid over the child this bridge owns; see [`crate::io::ProcessKill`].
    fn killer(&self) -> Option<Arc<dyn crate::io::AgentKill>> {
        Some(Arc::new(crate::io::ProcessKill::new(&self.child)))
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
fn write_input(
    stdin: &Arc<Mutex<Option<ChildStdin>>>,
    tx: &mpsc::Sender<Option<Framed>>,
    pending: &Pending,
    input: AgentInput,
) -> crate::Result<()> {
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
            write_line(stdin, &line)?;
            // A persistent, `--input-format stream-json` session never echoes
            // the prompt back on stdout — verified live against a real
            // `claude` process (`tests::live_two_turn_structured_session_
            // when_claude_available`): two turns, no `"type":"user"` line
            // either time, only the assistant's own `"type":"assistant"`
            // reply. (The one-shot `claude -p "prompt"` invocation is
            // different and does echo it — this bridge never uses that
            // form.) So the transcript's user block is synthesized here,
            // the moment the line actually reaches stdin, rather than
            // waiting on an echo that this mode never sends.
            for text in content.iter().filter_map(Content::as_text) {
                let _ = tx.send(Some((
                    AgentEvent::UserMessageChunk {
                        content: Content::text(text),
                        message_id: None,
                    },
                    None,
                )));
            }
            Ok(())
        }
        AgentInput::AnswerPermission {
            request_id,
            outcome,
            updated_input,
        } => {
            // Answering retires the ask, so a later cancel does not deny it a second time.
            let suggestions = pending
                .lock()
                .ok()
                .and_then(|mut p| p.remove(&request_id))
                .unwrap_or_default();
            let (behavior, updated_permissions) = match &outcome {
                PermissionOutcome::Selected { option_id } => answer(option_id, &suggestions),
                // A cancelled turn denies whatever was waiting on a human.
                PermissionOutcome::Cancelled => ("deny", None),
            };
            let line = control_response(
                &request_id,
                behavior,
                updated_input.unwrap_or_else(|| json!({})),
                updated_permissions,
            );
            write_line(stdin, &line)
        }
        AgentInput::Cancel => {
            // Answer first, interrupt after. Every outstanding ask is denied
            // (`PermissionOutcome::Cancelled` — the contract in
            // `_docs/io-modes.md` §"Permissions and cancellation"); interrupting with one still
            // unanswered makes Claude fail the tool with "Tool permission stream closed before
            // response received".
            deny_outstanding(stdin, pending);
            // Then the turn — and only the turn. `{"subtype":"interrupt"}` aborts what is running
            // and leaves the session open for the next prompt
            // (`_docs/harness/claude-code.md` §"Process lifecycle"). Claude answers it with a
            // `control_response`, which this bridge ignores: the abort itself shows up as the
            // turn's `result`, and the response carries nothing a consumer draws.
            write_line(stdin, &interrupt_request())
        }
        AgentInput::Shutdown => {
            // Teardown. Deny first for the same reason a cancel does, then close stdin so Claude
            // Code sees EOF and exits; the reader thread keeps draining stdout until the process
            // actually goes (`_docs/harness/claude-code.md` §"Process lifecycle").
            deny_outstanding(stdin, pending);
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
/// line to zero-or-more [`AgentEvent`]s and push them onto `tx`, and record
/// `can_use_tool` `control_request` in `pending` so the caller — nobody else — can answer it.
/// Every other `control_request` subtype it answers itself, with an error.
///
/// Returns (and drops `tx`, closing the channel) on stdout EOF or a channel
/// disconnect (nobody left to receive).
fn read_loop(
    stdout: ChildStdout,
    pending: Pending,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    tx: mpsc::Sender<Option<Framed>>,
) {
    read_stream(stdout, &pending, &stdin, &tx);
    // The reader thread is about to end no matter which path above got it
    // here — send the explicit "done" so `next_event` sees real EOF rather
    // than blocking on a channel `write_input`'s own clone keeps open.
    let _ = tx.send(None);
}

fn read_stream(
    stdout: ChildStdout,
    pending: &Pending,
    stdin: &Arc<Mutex<Option<ChildStdin>>>,
    tx: &mpsc::Sender<Option<Framed>>,
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

        // Record the ask *before* the event goes out, so a caller answering on another thread
        // the instant it sees the event finds it outstanding rather than unknown.
        if let Some((request_id, suggestions)) = permission_ask(&value) {
            if let Ok(mut guard) = pending.lock() {
                guard.insert(request_id, suggestions);
            }
        } else if value.get("type").and_then(Value::as_str) == Some("control_request") {
            // A subtype this bridge does not implement. Nobody downstream will ever answer it, and
            // an unanswered `control_request` stalls the turn for good, so say so now.
            if let Some(request_id) = value.get("request_id").and_then(Value::as_str) {
                let subtype = value
                    .get("request")
                    .and_then(|r| r.get("subtype"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                tracing::warn!(
                    subtype,
                    "unsupported claude control_request; answering error"
                );
                if write_line(stdin, &control_error(request_id, subtype)).is_err() {
                    return;
                }
            }
        }

        let raw: Arc<str> = Arc::from(line);
        for ev in mapper.map_event(&value) {
            tracing::debug!(event = ?ev, "claude event");
            if tx.send(Some((ev, Some(Arc::clone(&raw))))).is_err() {
                // No one is listening anymore.
                return;
            }
        }
    }
}

/// The `request_id` and `permission_suggestions` of a `can_use_tool`
/// `control_request`, or `None` for any other line — the same gate
/// [`map_control_request`] applies, so exactly what becomes an event becomes
/// an outstanding ask.
fn permission_ask(value: &Value) -> Option<(String, Vec<Value>)> {
    if value.get("type").and_then(Value::as_str) != Some("control_request") {
        return None;
    }
    let request = value.get("request")?;
    if request.get("subtype").and_then(Value::as_str) != Some("can_use_tool") {
        return None;
    }
    let request_id = value.get("request_id").and_then(Value::as_str)?;
    let suggestions = request
        .get("permission_suggestions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Some((request_id.to_string(), suggestions))
}

/// The `option_id` prefix of an "always" option, followed by the index of the
/// `permission_suggestions` entry it stands for — the encoding [`map_control_request`] writes and
/// [`answer`] reads, so answering needs no second mapping.
const ALLOW_ALWAYS: &str = "allow_always:";

/// The `behavior` (and any `updatedPermissions`) a chosen `option_id` means.
///
/// `allow`/`deny` are Claude's own `behavior` strings, so they pass straight through. An
/// `allow_always:<n>` echoes `permission_suggestions[n]` back as `updatedPermissions`, which is
/// how the suggestion ("switch this session to `acceptEdits`") is what makes the choice stick —
/// an unknown id is a `deny`, since running a tool nobody recognisably approved is the worse
/// failure.
fn answer(option_id: &str, suggestions: &[Value]) -> (&'static str, Option<Value>) {
    if option_id == "allow" {
        return ("allow", None);
    }
    if let Some(index) = option_id.strip_prefix(ALLOW_ALWAYS)
        && let Some(suggestion) = index.parse::<usize>().ok().and_then(|i| suggestions.get(i))
    {
        return ("allow", Some(json!([suggestion])));
    }
    ("deny", None)
}

/// Build the `control_response` NDJSON line
/// (`_docs/harness/claude-code.md` §"Tool approval in headless mode").
///
/// `updated_permissions` is only present when the caller took an "always" option; the field is
/// left off entirely otherwise, keeping the plain allow/deny line byte-identical to the shape
/// verified against 2.1.258.
fn control_response(
    request_id: &str,
    behavior: &str,
    updated_input: Value,
    updated_permissions: Option<Value>,
) -> Value {
    let mut response = json!({
        "behavior": behavior,
        "updatedInput": updated_input,
    });
    if let Some(updates) = updated_permissions
        && let Some(object) = response.as_object_mut()
    {
        object.insert("updatedPermissions".to_string(), updates);
    }
    json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": response,
        },
    })
}

/// Deny every permission ask still outstanding, and forget them — an ask answered once must not
/// be answered twice. Shared by [`AgentInput::Cancel`] and [`AgentInput::Shutdown`]: both leave
/// the turn, and a harness holding an unanswered ask has no timeout to fall back on.
fn deny_outstanding(stdin: &Arc<Mutex<Option<ChildStdin>>>, pending: &Pending) {
    let outstanding: Vec<String> = pending
        .lock()
        .map(|mut p| p.drain().map(|(id, _)| id).collect())
        .unwrap_or_default();
    for request_id in outstanding {
        let line = control_response(&request_id, "deny", json!({}), None);
        let _ = write_line(stdin, &line);
    }
}

/// Build the client→CLI `interrupt` `control_request` line — Claude Code's own
/// turn abort, which ends the turn and leaves the session open
/// (`_docs/harness/claude-code.md` §"Process lifecycle").
///
/// `reason` is an open set Claude forwards to the turn's `AbortSignal.reason`; tools branch on it,
/// and `interrupt` is the value that means "the human pressed stop", which suppresses the error
/// output a generic abort would print. `request_id` only has to be unique within the session, so a
/// process-wide counter is enough — no uuid dependency for a string nothing outside reads.
fn interrupt_request() -> Value {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    json!({
        "type": "control_request",
        "request_id": format!("am-interrupt-{n}"),
        "request": { "subtype": "interrupt", "reason": "interrupt" },
    })
}

/// Build the `control_response` that refuses a `control_request` subtype this bridge does not
/// implement. Claude only needs *an* answer to stop waiting; an error is the honest one.
fn control_error(request_id: &str, subtype: &str) -> Value {
    json!({
        "type": "control_response",
        "response": {
            "subtype": "error",
            "request_id": request_id,
            "error": format!("unsupported control_request subtype '{subtype}'"),
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
/// Claude's stream states most accounting **cumulatively for the session**, and states occupancy
/// only in passing, so a stateless mapper cannot report either honestly. This is what it takes to:
///
/// - `contextWindow` appears in `result.modelUsage` and nowhere else, so a window is learned at
///   the end of a turn and used by every turn after it;
/// - `modelUsage` and `total_cost_usd` are session totals, so this turn's spend and cost are the
///   difference from the last ones seen;
/// - occupancy belongs to the last *parent* assistant message, so a `result` and a subagent's line
///   both carry it forward rather than recomputing it (`_data` capture: seq 780 reported
///   `used = 35_436` and seq 781 `used = 218_336` for a context that never grew);
/// - subagent lines spend against the same session totals, so what they were seen to spend is
///   subtracted from the turn's delta, leaving the two sets of rows summing to the truth;
/// - one session can announce itself twice (capture seqs 741 and 748), and a transcript should say
///   so once.
#[derive(Default)]
struct Mapper {
    /// Each model's context window, learned from `result.modelUsage`.
    windows: HashMap<String, u64>,
    /// The occupancy the conversation itself last reported: the model that answered, and the
    /// tokens its message found sitting in the window.
    occupancy: Option<(String, u64)>,
    /// Session-cumulative spend per model, as `modelUsage` last stated it.
    spent: HashMap<String, Spend>,
    /// Session-cumulative cost per model; the empty key is the run's own `total_cost_usd`.
    costs: HashMap<String, f64>,
    /// Spend already reported as a subagent's, this turn, per model.
    subagent_spend: HashMap<String, Spend>,
    /// Tool calls that spawned a background agent and have not been told it ended.
    launched: Vec<String>,
    /// What each delegate is running as, by the spawning call's id: the model its launch resolved
    /// to, or failing that the one its first line named. Remembered rather than re-read, so every
    /// block of one delegate says the same thing instead of the answer depending on which block a
    /// consumer happened to read first.
    delegate_models: HashMap<String, String>,
    /// The session id already announced, and the mode it was announced with.
    started: Option<String>,
    mode: Option<String>,
}

impl Mapper {
    /// Map one parsed NDJSON stdout line to zero or more [`AgentEvent`]s.
    ///
    /// See the mapping table in `_docs/harness/claude-code.md`
    /// §"Output stream protocol".
    fn map_event(&mut self, value: &Value) -> Vec<AgentEvent> {
        match value.get("type").and_then(Value::as_str) {
            Some("system") if value.get("subtype").and_then(Value::as_str) == Some("init") => {
                self.map_init(value)
            }
            Some("assistant") => self.map_assistant(value),
            Some("user") => {
                let events = map_user(value);
                // The launch states what the spawn resolved to (`tool_use_result.resolvedModel`,
                // capture seq 753) — before the delegate has said anything of its own.
                let resolved = value
                    .get("tool_use_result")
                    .and_then(|r| r.get("resolvedModel"))
                    .and_then(Value::as_str);
                // A spawn's `tool_result` only says the agent was launched, so the call it names
                // is remembered until something says it ended — see [`Mapper::finish_launched`].
                for event in &events {
                    let AgentEvent::ToolCallUpdate { update } = event else {
                        continue;
                    };
                    if update.status != Some(ToolStatus::InProgress) {
                        continue;
                    }
                    if let Some(model) = resolved {
                        self.delegate_models
                            .insert(update.id.clone(), model.to_string());
                    }
                    self.launched.push(update.id.clone());
                }
                events
            }
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
            Some("rate_limit_event") => vec![map_rate_limit(value)],
            other => {
                tracing::trace!(kind = ?other, "claude event ignored");
                Vec::new()
            }
        }
    }

    /// A `system`/`init` event. **Once per session**: Claude sends this line again on a session it
    /// has already announced (capture seqs 741 and 748), and a second `SessionStarted` would draw
    /// a second conversation. A repeat that changed the mode is a mode change, and nothing else.
    fn map_init(&mut self, value: &Value) -> Vec<AgentEvent> {
        let event = init_event(value);
        let AgentEvent::SessionStarted {
            session_id, mode, ..
        } = &event
        else {
            return vec![event];
        };

        if self.started.is_some() && &self.started == session_id {
            let changed = mode.clone().filter(|mode| self.mode.as_ref() != Some(mode));
            self.mode = mode.clone();
            return changed
                .map(|current_mode_id| AgentEvent::CurrentModeUpdate { current_mode_id })
                .into_iter()
                .collect();
        }

        self.started = session_id.clone();
        self.mode = mode.clone();
        vec![event]
    }

    /// An `assistant` event: its content blocks, then its own token usage if
    /// a window for that model is already known.
    fn map_assistant(&mut self, value: &Value) -> Vec<AgentEvent> {
        let mut origin = origin_of(value);
        if let Some(parent) = origin.parent_tool_use_id.clone() {
            let named = value
                .get("message")
                .and_then(|m| m.get("model"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let known = self
                .delegate_models
                .entry(parent)
                .or_insert_with(|| named.to_string());
            origin.model = (!known.is_empty()).then(|| known.clone());
        }
        let mut events = map_content_blocks(value, false, &origin);
        if let Some(usage) = self.message_usage(value, &origin) {
            events.push(usage);
        }
        events
    }

    /// Per-message accounting, which is where **occupancy** comes from and the only place it does.
    ///
    /// What fills a context window is this message's fresh input plus everything it read from or
    /// wrote to the cache — a level, as of this message, that falls on compaction as legitimately
    /// as it rises. `output_tokens` is deliberately not in it: it is a streaming stub (`1`, `2`,
    /// `3` on the capture's assistant lines) and the real figure only arrives in `result`.
    ///
    /// A subagent's line reports what it spent and leaves occupancy where the conversation itself
    /// left it — its context is its own, and drawing it as the parent's is what made occupancy
    /// oscillate within one turn in the capture (31_076 → 17_259 → 33_816).
    fn message_usage(&mut self, value: &Value, origin: &Origin) -> Option<AgentEvent> {
        let message = value.get("message")?;
        let usage = message.get("usage")?;
        let model = message.get("model").and_then(Value::as_str)?;

        let field = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
        let spend = Spend {
            input: field("input_tokens"),
            cache_read: field("cache_read_input_tokens"),
            cache_creation: field("cache_creation_input_tokens"),
            ..Spend::default()
        };
        let occupied = spend.total();

        // Accounted under the canonical name, because `modelUsage` keys the same model by its
        // dated release — see [`canonical`].
        let key = canonical(model, None);
        if origin.is_subagent() {
            // Remembered so the turn's cumulative delta can be split between the conversation and
            // the subagents inside it, instead of counting this spend twice.
            let seen = self.subagent_spend.entry(key).or_default();
            *seen = seen.saturating_add(&spend);
        } else {
            self.occupancy = Some((key, occupied));
        }

        let (used, size) = self.ring();
        // A ratio with an invented denominator is worse than no ratio, so a model whose window is
        // still unknown reports nothing at all.
        (size > 0).then(|| AgentEvent::UsageUpdate {
            used,
            size,
            cost: None,
            model: Some(model.to_string()),
            spend: origin.is_subagent().then_some(spend),
            origin: origin.clone(),
        })
    }

    /// The context ring as it stands: the conversation's last occupancy, against the window of the
    /// model that reported it. `(0, 0)` before any parent message, which draws no ring.
    fn ring(&self) -> (u64, u64) {
        let Some((model, used)) = &self.occupancy else {
            return (0, 0);
        };
        match self.windows.get(model) {
            Some(size) => (*used, *size),
            None => (0, 0),
        }
    }

    /// A `result` event: the turn's spend — which is where a context window
    /// is learned — and then the turn's end.
    fn map_result(&mut self, value: &Value) -> Vec<AgentEvent> {
        let mut events = self.turn_usage(value);
        events.extend(self.finish_launched(value));

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

    /// End the turn's background agents.
    ///
    /// **Claude announces a spawn and never announces that one agent finished.** Its `tool_result`
    /// says `async_launched` and nothing later names that `tool_use_id` again; the subagent's own
    /// lines carry `parent_tool_use_id` but no last-line marker (capture seqs 751–779). What the
    /// stream does state is the tally on `result`: the capture's closing line reports
    /// `subagent_stats: {spawned: 3, completed: 3, failed: 0}` for the three calls left in
    /// progress, so the turn's end is where they can honestly be closed — and leaving them
    /// `InProgress` for ever, as the switcher above the composer showed, is the one answer that
    /// can never become true.
    ///
    /// A turn that ends with agents still running (`completed + failed < spawned`) keeps them in
    /// progress; the next `result` closes them.
    ///
    /// ponytail: the tally is per turn, not per call, so a mixed turn cannot say *which* agent
    /// failed — all of them read as failed only when none completed. Per-call truth needs a
    /// harness line that names the `tool_use_id`, and there is none.
    fn finish_launched(&mut self, value: &Value) -> Vec<AgentEvent> {
        if self.launched.is_empty() {
            return Vec::new();
        }
        let stats = value.get("subagent_stats");
        let stat = |name: &str| {
            stats
                .and_then(|s| s.get(name))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        let (completed, failed) = (stat("completed"), stat("failed"));
        if stats.is_some() && completed + failed < stat("spawned") {
            return Vec::new();
        }
        let status = if completed == 0 && failed > 0 {
            ToolStatus::Failed
        } else {
            ToolStatus::Completed
        };
        self.launched
            .drain(..)
            .map(|id| AgentEvent::ToolCallUpdate {
                update: ToolCallUpdate::finished(id, status),
            })
            .collect()
    }

    /// Read `modelUsage` — **camelCase on the wire**, whatever an older version of the harness
    /// contract said — remember each model's context window, and report **what this turn spent**.
    ///
    /// Every model gets a report, not just the busiest one: the capture bills haiku 901 in / 14 out
    /// / $0.000971 on a turn dominated by sonnet, and keeping only the largest entry dropped it.
    /// Every figure in `modelUsage` is cumulative for the session, so each is reported as the
    /// difference from the last one seen; a `result` moves no occupancy, it only carries forward
    /// what the last assistant message said.
    ///
    /// Falls back to the top-level `total_cost_usd`, for a run that reported no per-model
    /// breakdown; that path has no window and no token counts, so it can only contribute a cost.
    fn turn_usage(&mut self, value: &Value) -> Vec<AgentEvent> {
        let total_cost = value.get("total_cost_usd").and_then(Value::as_f64);

        let Some(model_usage) = value.get("modelUsage").and_then(Value::as_object) else {
            let (used, size) = self.ring();
            let Some(cost) = total_cost.and_then(|total| self.cost_delta("", total)) else {
                return Vec::new();
            };
            return vec![AgentEvent::UsageUpdate {
                used,
                size,
                cost: Some(cost),
                model: None,
                spend: None,
                origin: Origin::default(),
            }];
        };

        // Sorted so a turn's reports are in one order whatever the JSON map preserved. Each entry
        // carries the name it is billed under and the name it is accounted under.
        let mut models: Vec<(&String, String)> = model_usage
            .keys()
            .map(|model| (model, canonical(model, Some(&model_usage[model]))))
            .collect();
        models.sort();

        // Windows first, and only then the ring: this line is where a window is learned, and the
        // very first `result` of a run states the denominator for occupancy already reported
        // against it.
        for (model, key) in &models {
            let window = model_usage[*model]
                .get("contextWindow")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if window > 0 {
                self.windows.insert(key.clone(), window);
            }
        }
        let (used, size) = self.ring();

        // Taken once, whether or not it is used: the running total has to be recorded even when
        // per-model `costUSD` already accounts for it, or the next line that falls back to it
        // would report the whole session as its own.
        let mut run_cost = total_cost.and_then(|total| self.cost_delta("", total));

        // Which entry is "the" model of this turn, for the two things only one entry may carry:
        // the ring, which belongs to whatever model actually answered, and a run-level
        // `total_cost_usd` that no `costUSD` accounted for. Before any assistant message there is
        // no answering model, and the busiest entry is the best the line itself can say.
        let ring_model = self.occupancy.as_ref().map(|(model, _)| model.clone());
        let cost_model = ring_model
            .clone()
            .filter(|m| models.iter().any(|(_, key)| key == m))
            .or_else(|| {
                models
                    .iter()
                    .max_by_key(|(model, _)| {
                        model_usage[*model]
                            .get("inputTokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                    })
                    .map(|(_, key)| key.clone())
            });
        let mut events = Vec::new();
        for (model, key) in models {
            let usage = &model_usage[model];
            let field = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);

            // `outputTokens` includes the thinking tokens, so the two are separated rather than
            // added — `Spend::total` must not count a reasoning token twice.
            let thinking = field("thinkingTokens");
            let cumulative = Spend {
                input: field("inputTokens"),
                output: field("outputTokens").saturating_sub(thinking),
                thinking,
                cache_read: field("cacheReadInputTokens"),
                cache_creation: field("cacheCreationInputTokens"),
            };
            let previous = self
                .spent
                .insert(key.clone(), cumulative)
                .unwrap_or_default();
            let delta = cumulative.saturating_sub(&previous);
            // What the subagents of this turn already reported is theirs, not the conversation's.
            let subagents = self.subagent_spend.remove(&key).unwrap_or_default();
            let spend = delta.saturating_sub(&subagents);

            let cost = usage
                .get("costUSD")
                .and_then(Value::as_f64)
                .and_then(|total| self.cost_delta(&key, total))
                .or_else(|| {
                    (Some(key.clone()) == cost_model)
                        .then(|| run_cost.take())
                        .flatten()
                });

            let carries_occupancy = Some(key.clone()) == ring_model;
            if spend.total() == 0 && cost.is_none() && !carries_occupancy {
                continue;
            }
            // Occupancy is the conversation's, not the model's: every report on this line states
            // the same level, because a report that said `used: 0` for a model that answered
            // nothing was the last one a consumer kept, and emptied the ring.
            events.push(AgentEvent::UsageUpdate {
                used,
                size,
                cost,
                model: Some(model.clone()),
                spend: Some(spend),
                origin: Origin::default(),
            });
        }
        self.subagent_spend.clear();
        events
    }

    /// This report's cost: what the running total has grown by since the last one, since Claude
    /// only ever states the session's cumulative figure (0.0530078 then 0.184233 across the
    /// capture's two turns) and summing those over-counts every turn but the first.
    fn cost_delta(&mut self, key: &str, total: f64) -> Option<super::Cost> {
        let previous = self.costs.insert(key.to_string(), total).unwrap_or(0.0);
        let amount = (total - previous).max(0.0);
        (amount > 0.0).then(|| super::Cost {
            amount,
            currency: "USD".to_string(),
        })
    }
}

/// The name a model is accounted under.
///
/// `modelUsage` keys a model by its dated release (`claude-haiku-4-5-20251001`, capture seq 744)
/// while an assistant message names the canonical alias (`claude-haiku-4-5`) — so a window learned
/// from one is never found for the other, occupancy has no denominator, and the ring sits at zero
/// for the whole session however many turns complete. `canonicalModel` says it where the harness
/// states it; a trailing `-YYYYMMDD` is stripped where it does not.
fn canonical(model: &str, entry: Option<&Value>) -> String {
    if let Some(name) = entry
        .and_then(|e| e.get("canonicalModel"))
        .and_then(Value::as_str)
    {
        return name.to_string();
    }
    match model.rsplit_once('-') {
        Some((head, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => {
            head.to_string()
        }
        _ => model.to_string(),
    }
}

/// Who produced a line: `parent_tool_use_id` is set on everything a subagent said, and
/// `subagent_type`/`task_description` name which one (capture seqs 757, 764, 766).
fn origin_of(value: &Value) -> Origin {
    let string = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
    Origin {
        parent_tool_use_id: string("parent_tool_use_id"),
        subagent_type: string("subagent_type"),
        // Filled by [`Mapper::map_assistant`] from what the spawn resolved to; nothing on the
        // line itself states either.
        model: None,
        thinking: None,
    }
}

/// A `system`/`init` line's contents: the session's id and everything it can do. Whether it is
/// announced at all is [`Mapper::map_init`]'s decision.
fn init_event(value: &Value) -> AgentEvent {
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

/// A `rate_limit_event`: how full the account's rolling windows are, and when they reset. Only
/// `unifiedWindows.{five_hour,seven_day}`, the top-level `status` and the two overage fields are
/// read — `rateLimitType`, `isUsingOverage` and `uuid` have no reader yet.
fn map_rate_limit(value: &Value) -> AgentEvent {
    let info = value.get("rate_limit_info");
    let window = |name: &str| {
        info.and_then(|i| i.get("unifiedWindows"))
            .and_then(|windows| windows.get(name))
            .and_then(|window| {
                let utilization = window.get("utilization").and_then(Value::as_f64)?;
                let resets_at = window.get("resetsAt").and_then(Value::as_i64)?;
                Some(RateLimitWindow {
                    utilization_pct: (utilization * 100.0).round().clamp(0.0, 100.0) as u8,
                    resets_at,
                })
            })
    };

    let string = |key: &str| {
        info.and_then(|i| i.get(key))
            .and_then(Value::as_str)
            .map(str::to_string)
    };

    AgentEvent::RateLimitUpdate {
        five_hour: window("five_hour"),
        seven_day: window("seven_day"),
        status: info
            .and_then(|i| i.get("status"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        overage_status: string("overageStatus"),
        overage_reason: string("overageDisabledReason"),
    }
}

/// A `user` event: what the harness received from us, and any tool results
/// carried back on the same turn.
fn map_user(value: &Value) -> Vec<AgentEvent> {
    map_content_blocks(value, true, &Origin::default())
}

/// Map an `assistant` or `user` event's `message.content` blocks.
///
/// `is_user` selects which block shapes are expected (`tool_result` and the
/// user's own text for user messages; `text`/`thinking`/`tool_use` for
/// assistant messages) — unrecognized block types are ignored either way, so
/// passing the wrong flag only means missing events, not a panic.
fn map_content_blocks(value: &Value, is_user: bool, origin: &Origin) -> Vec<AgentEvent> {
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
                            origin: origin.clone(),
                        }
                    });
                }
            }
            Some("thinking") if !is_user => {
                // A signature with no text is not a thought. Claude withholds the reasoning and
                // sends `{"thinking":"","signature":"EvEMCqgB…"}` (capture seqs 749, 774); the
                // turn really did think, and that count arrives in `result` as `Spend::thinking`,
                // so an empty block here would only draw an empty box.
                match block.get("thinking").and_then(Value::as_str) {
                    Some(text) if !text.is_empty() => {
                        events.push(AgentEvent::AgentThoughtChunk {
                            content: Content::text(text),
                            message_id: message_id.clone(),
                            origin: origin.clone(),
                        });
                    }
                    _ => {}
                }
            }
            Some("tool_use") if !is_user => {
                let mut call = map_tool_use(block);
                call.origin = origin.clone();
                events.push(AgentEvent::ToolCall { call });
            }
            Some("tool_result") if is_user => {
                events.push(AgentEvent::ToolCallUpdate {
                    update: map_tool_result(block, value.get("tool_use_result")),
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
    let id = block.get("id").and_then(Value::as_str).unwrap_or(name);
    let mut call = tool_call(name, id, block.get("input"));
    call.status = ToolStatus::InProgress;
    call
}

/// One tool call, from the three things every shape that describes one carries: its name, its id,
/// and its input.
///
/// Shared by [`map_tool_use`] and [`map_control_request`] because a `can_use_tool`
/// `control_request` describes the *same* call in the *same* field names (`file_path`, `path`,
/// `command`, `pattern`, `url`) — it just spells the name `tool_name` and the id `tool_use_id`.
/// The caller sets the status: a call Claude is asking about has not started.
fn tool_call(name: &str, id: &str, input: Option<&Value>) -> ToolCall {
    let string = |key: &str| input.and_then(|i| i.get(key)).and_then(Value::as_str);

    let kind = tool_kind(name);
    let path = string("file_path").or_else(|| string("path"));
    let target = path
        .or_else(|| string("command"))
        .or_else(|| string("pattern"))
        .or_else(|| string("url"));

    // A `Task`/`Agent` call's target is a subagent, and its one readable name is the description
    // the caller wrote ("Formal greeting agent", capture seq 751) — the name and `subagent_type`
    // alone say nothing about what was delegated.
    let title = match (string("description"), target) {
        (Some(what), _) if matches!(name, "Task" | "Agent") => what.to_string(),
        (_, Some(target)) => format!("{name} {target}"),
        _ => name.to_string(),
    };

    let mut call = ToolCall::new(id, title);
    call.kind = kind;
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
        // A spawn is a delegation, not a thought: it starts a second transcript, and the switcher
        // above the composer reads this kind to find the agents a turn launched.
        "Task" | "Agent" => ToolKind::Delegate,
        "TodoWrite" | "ExitPlanMode" => ToolKind::Think,
        _ => ToolKind::Other,
    }
}

/// A `tool_result` block: the call named by `tool_use_id` has finished.
///
/// A `status` of `async_launched` is the one progress signal between a tool starting and
/// finishing, so it stays in progress rather than completing — Claude states it on the block or,
/// for a spawned agent, on the line's sibling `tool_use_result` (capture seq 753).
///
/// Such a result's text is ~800 characters of harness plumbing addressed to the model — an agent
/// id, a transcript path and the instruction not to mention either — so it is left in `raw_output`
/// rather than forwarded as something a transcript would draw.
fn map_tool_result(block: &Value, result: Option<&Value>) -> ToolCallUpdate {
    let failed = block
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let launched = block.get("status").and_then(Value::as_str) == Some("async_launched")
        || result.and_then(|r| r.get("status")).and_then(Value::as_str) == Some("async_launched")
        || result
            .and_then(|r| r.get("isAsync"))
            .and_then(Value::as_bool)
            == Some(true);

    let status = match (failed, launched) {
        (true, _) => ToolStatus::Failed,
        (_, true) => ToolStatus::InProgress,
        _ => ToolStatus::Completed,
    };

    let content = (!launched)
        .then(|| block.get("content").and_then(result_text))
        .flatten()
        .map(|text| {
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

/// Map a `can_use_tool` `control_request` (`_docs/harness/claude-code.md`
/// §"Tool approval in headless mode") to an [`AgentEvent::PermissionRequest`],
/// carrying the whole tool call so a dialog can show what it is authorising.
///
/// The request describes the call inline — `tool_name`, `input`, and the
/// `tool_use_id` of the `tool_use` block already in the transcript. That id is
/// the event's id, so a consumer joins the ask to the call it can already see
/// rather than drawing a second one; only a request without one falls back to
/// the `request_id`. Any other `control_request` subtype yields no event: this
/// bridge answers what it understands and nothing else.
///
/// `allow`/`deny` are Claude's own `behavior` strings, so those two ids pass
/// straight through [`answer`]. Each `{"type":"setMode",…}` entry in
/// `permission_suggestions` adds one [`PermissionKind::AllowAlways`] option
/// whose id names that entry, and choosing it sends the suggestion back as
/// `updatedPermissions` — the "always" is Claude's own, not a memory this
/// bridge would have to keep.
fn map_control_request(value: &Value) -> Vec<AgentEvent> {
    let Some((request_id, suggestions)) = permission_ask(value) else {
        return Vec::new();
    };
    // `permission_ask` already read it; a `None` here is unreachable.
    let Some(request) = value.get("request") else {
        return Vec::new();
    };
    let name = request
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("tool");
    let id = request
        .get("tool_use_id")
        .and_then(Value::as_str)
        .unwrap_or(&request_id);
    let mut call = tool_call(name, id, request.get("input"));
    call.status = ToolStatus::Pending;

    let mut options = vec![PermissionOption {
        option_id: "allow".to_string(),
        name: "Allow".to_string(),
        kind: PermissionKind::AllowOnce,
    }];
    for (index, suggestion) in suggestions.iter().enumerate() {
        if suggestion.get("type").and_then(Value::as_str) != Some("setMode") {
            continue;
        }
        let Some(mode) = suggestion.get("mode").and_then(Value::as_str) else {
            continue;
        };
        options.push(PermissionOption {
            option_id: format!("{ALLOW_ALWAYS}{index}"),
            name: format!("Allow, and switch to {mode}"),
            kind: PermissionKind::AllowAlways,
        });
    }
    options.push(PermissionOption {
        option_id: "deny".to_string(),
        name: "Deny".to_string(),
        kind: PermissionKind::RejectOnce,
    });

    vec![AgentEvent::PermissionRequest {
        request_id,
        tool_call: ToolCallUpdate {
            id: call.id,
            title: Some(call.title),
            kind: Some(call.kind),
            status: Some(call.status),
            content: (!call.content.is_empty()).then_some(call.content),
            locations: (!call.locations.is_empty()).then_some(call.locations),
            // `tool_call` already read the request's `input` into `raw_input`; carry it
            // through rather than dropping it, since this is a request, not a result.
            raw_input: call.raw_input,
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
                origin: Origin::default(),
            }
        );
        assert_eq!(
            events[1],
            AgentEvent::AgentThoughtChunk {
                content: Content::text("pondering"),
                message_id: Some("m1".to_string()),
                origin: Origin::default(),
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
    ///
    /// The expected numbers changed with the occupancy/spend split: `used` is no longer summed out
    /// of `modelUsage` (that is session billing, not what sits in the window — defect 1), so a
    /// `result` with no assistant message before it moves no ring at all, and what it reports is
    /// the turn's [`Spend`].
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
                // No assistant message has reported occupancy, so there is none to carry
                // forward — and a level nothing has stated draws no ring rather than an empty one.
                used: 0,
                size: 0,
                cost: Some(super::super::Cost {
                    amount: 0.5,
                    currency: "USD".to_string(),
                }),
                model: Some("claude-opus-5".to_string()),
                spend: Some(Spend {
                    input: 100,
                    output: 20,
                    thinking: 0,
                    cache_read: 900,
                    cache_creation: 0,
                }),
                origin: Origin::default(),
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
    ///
    /// Defect 7: the expected total changed because `output_tokens` is no longer added to
    /// anything. Claude sends `1`/`2`/`3` there on a streaming assistant line and the real figure
    /// only in `result`, so a per-message report carries occupancy and no spend.
    #[test]
    fn per_message_usage_waits_until_a_window_is_known() {
        let mut mapper = Mapper::default();
        let assistant: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5",
                "usage":{"input_tokens":10,"cache_read_input_tokens":90,"output_tokens":2},
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
                spend: None,
                origin: Origin::default(),
            }
        );
    }

    /// Defect 4: `modelUsage` names every model the turn used, and keeping only the busiest one
    /// dropped the rest. The capture (seq 781) bills haiku 901 in / 14 out / $0.000971 on a turn
    /// dominated by sonnet. Every report carries the same occupancy: it is the conversation's
    /// level, and a `used: 0` on the model that answered nothing was what emptied the ring
    /// (defect 1) once a consumer kept the last report it was given.
    #[test]
    fn every_model_in_a_result_reports_its_own_spend() {
        let mut mapper = Mapper::default();
        mapper.windows.insert("sonnet".to_string(), 1_000_000);
        mapper.occupancy = Some(("sonnet".to_string(), 31_000));

        let value: Value = serde_json::from_str(
            r#"{"type":"result","subtype":"success","total_cost_usd":0.184233,"modelUsage":{
                "haiku":{"inputTokens":901,"outputTokens":14,"costUSD":0.000971,
                  "contextWindow":200000},
                "sonnet":{"inputTokens":16,"outputTokens":1586,"thinkingTokens":442,
                  "cacheReadInputTokens":175250,"cacheCreationInputTokens":43070,
                  "costUSD":0.183262,"contextWindow":1000000}}}"#,
        )
        .unwrap();
        let events = mapper.map_event(&value);

        let usage: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::UsageUpdate {
                    used,
                    size,
                    cost,
                    model,
                    spend,
                    ..
                } => Some((model.clone(), *used, *size, cost.clone(), *spend)),
                _ => None,
            })
            .collect();
        assert_eq!(usage.len(), 2, "both models report: {events:?}");

        let (model, used, size, cost, spend) = &usage[0];
        assert_eq!(model.as_deref(), Some("haiku"));
        assert_eq!(
            (*used, *size),
            (31_000, 1_000_000),
            "occupancy is the conversation's, so every report on the line states the same level"
        );
        assert_eq!(cost.as_ref().map(|c| c.amount), Some(0.000971));
        assert_eq!(spend.unwrap().input, 901);

        let (model, used, size, cost, spend) = &usage[1];
        assert_eq!(model.as_deref(), Some("sonnet"));
        assert_eq!(
            (*used, *size),
            (31_000, 1_000_000),
            "a result carries occupancy forward, it never recomputes it"
        );
        assert_eq!(cost.as_ref().map(|c| c.amount), Some(0.183262));
        let spend = spend.unwrap();
        // Defect 8: `thinkingTokens` had no reader. It is part of `outputTokens` on the wire, so
        // the two are split rather than added.
        assert_eq!(spend.thinking, 442);
        assert_eq!(spend.output, 1586 - 442);
        // Defect 5: cache read and cache creation are separate; `cached()` is the read alone.
        assert_eq!(spend.cache_read, 175_250);
        assert_eq!(spend.cache_creation, 43_070);
    }

    /// Defect 6: every figure in a `result` is cumulative for the session (the capture's two turns
    /// report $0.0530078 then $0.184233 for the same run), so a second turn must report the
    /// difference — summing the raw figures over-counts every turn but the first.
    #[test]
    fn a_second_result_reports_the_difference_not_the_running_total() {
        let turn = |cost: &str, input: u64| -> Value {
            serde_json::from_str(&format!(
                r#"{{"type":"result","subtype":"success","total_cost_usd":{cost},"modelUsage":{{
                    "sonnet":{{"inputTokens":{input},"outputTokens":0,"contextWindow":1000000}}}}}}"#
            ))
            .unwrap()
        };
        let mut mapper = Mapper::default();
        mapper.map_event(&turn("0.0530078", 100));
        let events = mapper.map_event(&turn("0.184233", 250));

        let AgentEvent::UsageUpdate { cost, spend, .. } = &events[0] else {
            panic!("expected a usage update, got {events:?}");
        };
        let amount = cost.as_ref().unwrap().amount;
        assert!(
            (amount - (0.184233 - 0.0530078)).abs() < 1e-9,
            "the second turn cost the difference, got {amount}"
        );
        assert_eq!(spend.unwrap().input, 150);
    }

    /// Defect 2: everything a subagent produced is stamped `parent_tool_use_id` (capture seqs 757,
    /// 764, 766). Its speech is its own, and — the reason occupancy oscillated 31_076 → 17_259 →
    /// 33_816 within one turn — **its context is not the parent's**.
    #[test]
    fn a_subagent_reports_its_spend_and_never_moves_the_ring() {
        let mut mapper = Mapper::default();
        mapper
            .windows
            .insert("claude-sonnet-5".to_string(), 1_000_000);

        let parent: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"id":"m1","model":"claude-sonnet-5",
                "usage":{"input_tokens":2,"cache_read_input_tokens":30982,
                  "cache_creation_input_tokens":92,"output_tokens":2},
                "content":[]},"parent_tool_use_id":null}"#,
        )
        .unwrap();
        let subagent: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"id":"m2","model":"claude-sonnet-5",
                "usage":{"input_tokens":2,"cache_read_input_tokens":0,
                  "cache_creation_input_tokens":17257,"output_tokens":3},
                "content":[{"type":"text","text":"Good day, Marco"}]},
                "parent_tool_use_id":"toolu_015XvW4DQqg9Bmkmgvq9Fiwz",
                "subagent_type":"general-purpose","task_description":"Formal greeting agent"}"#,
        )
        .unwrap();

        let events = mapper.map_event(&parent);
        let AgentEvent::UsageUpdate { used, .. } = &events[0] else {
            panic!("expected usage, got {events:?}");
        };
        assert_eq!(*used, 31_076);

        let events = mapper.map_event(&subagent);
        assert_eq!(
            events[0],
            AgentEvent::AgentMessageChunk {
                content: Content::text("Good day, Marco"),
                message_id: Some("m2".to_string()),
                origin: Origin {
                    parent_tool_use_id: Some("toolu_015XvW4DQqg9Bmkmgvq9Fiwz".to_string()),
                    subagent_type: Some("general-purpose".to_string()),
                    model: Some("claude-sonnet-5".to_string()),
                    thinking: None,
                },
            }
        );
        let AgentEvent::UsageUpdate {
            used,
            spend,
            origin,
            ..
        } = &events[1]
        else {
            panic!("expected usage, got {events:?}");
        };
        assert_eq!(*used, 31_076, "the parent's ring is untouched");
        assert_eq!(spend.unwrap().cache_creation, 17_257);
        assert!(origin.is_subagent());

        // And the turn's cumulative total is split, so the two sets of rows sum to what was billed
        // rather than counting the subagent twice.
        let result: Value = serde_json::from_str(
            r#"{"type":"result","subtype":"success","modelUsage":{
                "claude-sonnet-5":{"inputTokens":4,"cacheCreationInputTokens":17349,
                  "cacheReadInputTokens":30982,"outputTokens":5,"contextWindow":1000000}}}"#,
        )
        .unwrap();
        let events = mapper.map_event(&result);
        let AgentEvent::UsageUpdate { spend, .. } = &events[0] else {
            panic!("expected usage, got {events:?}");
        };
        assert_eq!(
            spend.unwrap().cache_creation,
            92,
            "17_349 billed minus the 17_257 already reported as the subagent's"
        );
    }

    /// Defect 3: Claude withholds the reasoning and sends the signature alone (capture seqs 749,
    /// 774). `Value::as_str` is satisfied by `""`, so an empty thinking block became an empty
    /// thinking box; the turn's real reasoning count arrives in `result` as `Spend::thinking`.
    #[test]
    fn a_signature_only_thinking_block_is_not_a_thought() {
        let events = map(r#"{"type":"assistant","message":{"id":"m1","content":[
                {"type":"thinking","thinking":"","signature":"EvEMCqgBCBEYAipAMd2t"}]}}"#);
        assert!(events.is_empty(), "expected no events, got {events:?}");
    }

    /// Defect 10: one session announces itself twice (capture seqs 741 and 748). A second
    /// `SessionStarted` would draw a second conversation; a changed mode is a mode change.
    #[test]
    fn a_session_announces_itself_once() {
        let line = |mode: &str| -> Value {
            serde_json::from_str(&format!(
                r#"{{"type":"system","subtype":"init","session_id":"87f042f7",
                    "model":"claude-sonnet-5","permissionMode":"{mode}"}}"#
            ))
            .unwrap()
        };
        let mut mapper = Mapper::default();
        assert_eq!(mapper.map_event(&line("bypassPermissions")).len(), 1);
        assert!(mapper.map_event(&line("bypassPermissions")).is_empty());
        assert_eq!(
            mapper.map_event(&line("plan")),
            vec![AgentEvent::CurrentModeUpdate {
                current_mode_id: "plan".to_string(),
            }]
        );
    }

    /// Defect 9: a spawned agent's call is named by what it was asked to do (capture seq 751), and
    /// its result is ~800 characters of harness plumbing addressed to the model — an agent id, a
    /// transcript path, "do not mention to user" — which is not conversation (seq 753).
    #[test]
    fn a_spawned_agent_is_named_by_its_task_and_leaks_no_plumbing() {
        let events = map(r#"{"type":"assistant","message":{"id":"m1","content":[
                {"type":"tool_use","id":"toolu_015X","name":"Agent","input":{
                  "description":"Formal greeting agent","prompt":"Write a greeting.",
                  "subagent_type":"general-purpose"}}]}}"#);
        let AgentEvent::ToolCall { call } = &events[0] else {
            panic!("expected a tool call, got {events:?}");
        };
        assert_eq!(call.title, "Formal greeting agent");
        // Defect 2: a spawn is a delegation, not a thought — the transcript drew "THINK".
        assert_eq!(call.kind, ToolKind::Delegate);

        let events = map(
            r#"{"type":"user","message":{"content":[{"type":"tool_result",
                  "tool_use_id":"toolu_015X",
                  "content":[{"type":"text",
                    "text":"Async agent launched. agentId: a0537b18 (do not mention to user)"}]}]},
                "tool_use_result":{"isAsync":true,"status":"async_launched",
                  "agentId":"a0537b1804b6b5c33"}}"#,
        );
        let AgentEvent::ToolCallUpdate { update } = &events[0] else {
            panic!("expected an update, got {events:?}");
        };
        assert_eq!(update.status, Some(ToolStatus::InProgress));
        assert_eq!(update.content, None, "harness plumbing is not transcript");
        assert!(update.raw_output.is_some(), "but it is still available raw");
    }

    /// Defect 1: the ring stayed at `0.0K` for a whole live session while spend accumulated.
    ///
    /// Two things kept it there, and this replays both in the capture's own shapes. `modelUsage`
    /// keys a model by its dated release while an assistant message names the canonical alias
    /// (capture seq 744 bills `claude-haiku-4-5-20251001`), so the window was learned under a name
    /// occupancy was never looked up by — no denominator, no ring, and every `result` report then
    /// said `used: 0`. And a report for a model that answered nothing said `used: 0` too, which is
    /// the last report a consumer keeps.
    #[test]
    fn occupancy_is_reported_and_moves_across_turns_with_subagents() {
        let mut mapper = Mapper::default();
        let mut map = |json: &str| mapper.map_event(&serde_json::from_str::<Value>(json).unwrap());

        // Turn one. The message states occupancy; no window is known yet, so it draws no ring.
        let events = map(
            r#"{"type":"assistant","message":{"id":"m1","model":"claude-sonnet-5","content":[
                  {"type":"text","text":"hi"}],
                "usage":{"input_tokens":2,"cache_creation_input_tokens":11978,
                  "cache_read_input_tokens":19004,"output_tokens":2}}}"#,
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AgentEvent::UsageUpdate { .. })),
            "no window is known yet: {events:?}"
        );

        // The `result` is where the window is learned — under the dated key.
        let result = r#"{"type":"result","subtype":"success","total_cost_usd":0.053,"modelUsage":{
              "claude-haiku-4-5-20251001":{"inputTokens":901,"outputTokens":14,"costUSD":0.000971,
                "contextWindow":200000,"canonicalModel":"claude-haiku-4-5"},
              "claude-sonnet-5-20251101":{"inputTokens":2,"outputTokens":32,
                "cacheReadInputTokens":19004,"cacheCreationInputTokens":11978,"costUSD":0.052,
                "contextWindow":1000000,"canonicalModel":"claude-sonnet-5"}}}"#;
        let rings = |events: &[AgentEvent]| -> Vec<(u64, u64)> {
            events
                .iter()
                .filter_map(|e| match e {
                    AgentEvent::UsageUpdate { used, size, .. } => Some((*used, *size)),
                    _ => None,
                })
                .collect()
        };
        let first = rings(&map(result));
        assert_eq!(
            first,
            vec![(30_984, 1_000_000), (30_984, 1_000_000)],
            "both models report the conversation's one level, haiku included"
        );

        // Turn two: the level moves, and now it is reported per message.
        let second = rings(&map(
            r#"{"type":"assistant","message":{"id":"m2","model":"claude-sonnet-5","content":[
                  {"type":"text","text":"ok"}],
                "usage":{"input_tokens":2,"cache_creation_input_tokens":2740,
                  "cache_read_input_tokens":31074,"output_tokens":1}}}"#,
        ));
        assert_eq!(second, vec![(33_816, 1_000_000)], "the ring moved");

        // A subagent's line repeats the parent's level rather than its own (capture seq 758,
        // where a subagent's 17_259 was drawn as the conversation's).
        let sub = map(r#"{"type":"assistant","parent_tool_use_id":"toolu_015X",
                "subagent_type":"general-purpose",
                "message":{"id":"m3","model":"claude-sonnet-5","content":[
                  {"type":"text","text":"done"}],
                "usage":{"input_tokens":2,"cache_creation_input_tokens":17257,
                  "cache_read_input_tokens":0,"output_tokens":3}}}"#);
        assert_eq!(rings(&sub), vec![(33_816, 1_000_000)], "the parent's level");

        // And the turn's own `result` carries it forward, for every model on the line.
        let third = rings(&map(
            r#"{"type":"result","subtype":"success","total_cost_usd":0.184233,"modelUsage":{
                  "claude-sonnet-5-20251101":{"inputTokens":16,"outputTokens":1586,
                    "thinkingTokens":442,"cacheReadInputTokens":175250,
                    "cacheCreationInputTokens":43070,"costUSD":0.183262,
                    "contextWindow":1000000,"canonicalModel":"claude-sonnet-5"}}}"#,
        ));
        assert_eq!(third, vec![(33_816, 1_000_000)]);
        assert!(
            third
                .iter()
                .chain(&second)
                .chain(&first)
                .all(|(used, _)| *used > 0),
            "no report empties the ring"
        );
    }

    /// What a delegate is running as travels with every line it produced, and says the same thing
    /// on all of them: the model is learned once, from the launch's `resolvedModel` (capture seq
    /// 753), so a reader's answer does not depend on which block it happened to read.
    ///
    /// Thinking is absent because nothing states it: the spawn's input names a description, a
    /// prompt and a `subagent_type`, and the launch result adds only the model.
    #[test]
    fn a_delegates_lines_all_name_the_model_the_spawn_resolved_to() {
        let mut mapper = Mapper::default();
        let mut map = |json: &str| mapper.map_event(&serde_json::from_str::<Value>(json).unwrap());

        map(
            r#"{"type":"user","message":{"content":[{"type":"tool_result",
                "tool_use_id":"toolu_015X","content":"Async agent launched successfully."}]},
              "tool_use_result":{"isAsync":true,"status":"async_launched",
                "resolvedModel":"claude-sonnet-5","description":"Formal greeting agent"}}"#,
        );

        // A line the delegate produced. Its own `message.model` says the same, but the marker is
        // what the mapper remembered — a later line that named nothing would still carry it.
        let events = map(r#"{"type":"assistant","parent_tool_use_id":"toolu_015X",
                "subagent_type":"general-purpose",
                "message":{"id":"m2","model":"claude-sonnet-5","content":[
                  {"type":"text","text":"Good day, Marco"}]}}"#);
        let AgentEvent::AgentMessageChunk { origin, .. } = &events[0] else {
            panic!("expected a chunk, got {events:?}");
        };
        assert_eq!(origin.model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(origin.thinking, None, "no harness states it per delegate");

        let events = map(r#"{"type":"assistant","parent_tool_use_id":"toolu_015X",
                "subagent_type":"general-purpose",
                "message":{"id":"m3","content":[{"type":"text","text":"and again"}]}}"#);
        let AgentEvent::AgentMessageChunk { origin, .. } = &events[0] else {
            panic!("expected a chunk, got {events:?}");
        };
        assert_eq!(
            origin.model.as_deref(),
            Some("claude-sonnet-5"),
            "every block of one delegate says the same thing"
        );

        // A line of the conversation's own is not a delegate's, whatever it names.
        let events = map(
            r#"{"type":"assistant","message":{"id":"m4","model":"claude-sonnet-5",
                "content":[{"type":"text","text":"mine"}]}}"#,
        );
        let AgentEvent::AgentMessageChunk { origin, .. } = &events[0] else {
            panic!("expected a chunk, got {events:?}");
        };
        assert_eq!(origin.model, None);
    }

    /// Defect 2: Claude names the spawn tool `Task` as well as `Agent`, and both are delegations.
    #[test]
    fn both_spawn_tool_names_are_delegations() {
        assert_eq!(tool_kind("Task"), ToolKind::Delegate);
        assert_eq!(tool_kind("Agent"), ToolKind::Delegate);
        assert_eq!(tool_kind("TodoWrite"), ToolKind::Think);
    }

    /// Defect 3: a spawned agent's call sat at `InProgress` for ever, because the launch is the
    /// last thing Claude says about that `tool_use_id`. The turn's `result` is what says the
    /// agents ended (capture: `subagent_stats: {spawned: 3, completed: 3}`).
    #[test]
    fn a_spawned_call_ends_when_the_turn_accounts_for_it() {
        let mut mapper = Mapper::default();
        let mut map = |json: &str| mapper.map_event(&serde_json::from_str::<Value>(json).unwrap());

        map(
            r#"{"type":"user","message":{"content":[{"type":"tool_result",
                "tool_use_id":"toolu_015X","content":"Async agent launched successfully."}]},
              "tool_use_result":{"isAsync":true,"status":"async_launched"}}"#,
        );

        // Still running at the end of this turn: nothing is claimed.
        let events = map(
            r#"{"type":"result","subtype":"success","subagent_stats":{"spawned":1,
                "completed":0,"failed":0}}"#,
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolCallUpdate { .. })),
            "an agent that has not finished is not finished: {events:?}"
        );

        // The next `result` accounts for it.
        let events = map(
            r#"{"type":"result","subtype":"success","subagent_stats":{"spawned":1,
                "completed":1,"failed":0}}"#,
        );
        assert_eq!(
            events[0],
            AgentEvent::ToolCallUpdate {
                update: ToolCallUpdate::finished("toolu_015X", ToolStatus::Completed),
            },
            "got {events:?}"
        );
        assert_eq!(
            mapper
                .map_event(
                    &serde_json::from_str::<Value>(
                        r#"{"type":"result","subtype":"success","subagent_stats":{"spawned":1,
                    "completed":1,"failed":0}}"#
                    )
                    .unwrap()
                )
                .len(),
            1,
            "and only once — the turn ends, nothing else"
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

    /// The exact `can_use_tool` shape captured live against Claude Code 2.1.258 — there is no
    /// `request.tool_use` object: the call is described inline, and `tool_use_id` is the id of the
    /// `tool_use` block already in the transcript.
    #[test]
    fn a_control_request_carries_the_call_it_wants_authorised() {
        let events = map(r#"{"type":"control_request","request_id":"r1","request":{
                "subtype":"can_use_tool","tool_name":"Write","display_name":"Write",
                "input":{"file_path":"/tmp/a","content":"x"},"description":"a",
                "permission_suggestions":[
                    {"type":"setMode","mode":"acceptEdits","destination":"session"}],
                "tool_use_id":"t1"}}"#);
        let AgentEvent::PermissionRequest {
            request_id,
            tool_call,
            options,
        } = &events[0]
        else {
            panic!("expected a permission request, got {events:?}");
        };
        assert_eq!(request_id, "r1");
        // The transcript's own id, so a consumer joins the ask to the call it can already see.
        assert_eq!(tool_call.id, "t1");
        assert_eq!(tool_call.kind, Some(ToolKind::Edit));
        assert_eq!(tool_call.title.as_deref(), Some("Write /tmp/a"));
        assert_eq!(tool_call.status, Some(ToolStatus::Pending));
        // A `Write`'s new text is right there in the input, so the ask can show the diff.
        assert_eq!(
            tool_call.content,
            Some(vec![ToolContent::Diff {
                path: "/tmp/a".to_string(),
                old_text: None,
                new_text: "x".to_string(),
            }])
        );
        // Allow, the one "always" the suggestion offers, then deny.
        assert_eq!(options[0].option_id, "allow");
        assert_eq!(options[0].kind, PermissionKind::AllowOnce);
        assert_eq!(options[1].option_id, "allow_always:0");
        assert_eq!(options[1].kind, PermissionKind::AllowAlways);
        assert_eq!(options[2].option_id, "deny");
        assert_eq!(options[2].kind, PermissionKind::RejectOnce);
    }

    /// No suggestion, no "always": an option the answer path could not honor is not offered.
    #[test]
    fn a_control_request_without_suggestions_offers_only_allow_and_deny() {
        let events = map(r#"{"type":"control_request","request_id":"r1","request":{
                "subtype":"can_use_tool","tool_name":"Bash",
                "input":{"command":"echo hi"},"tool_use_id":"t1"}}"#);
        let AgentEvent::PermissionRequest { options, .. } = &events[0] else {
            panic!("expected a permission request, got {events:?}");
        };
        let ids: Vec<&str> = options.iter().map(|o| o.option_id.as_str()).collect();
        assert_eq!(ids, ["allow", "deny"]);
    }

    /// A request with no `tool_use_id` still has an id a caller can answer with.
    #[test]
    fn a_control_request_without_a_tool_use_id_falls_back_to_the_request_id() {
        let events = map(r#"{"type":"control_request","request_id":"r1","request":{
                "subtype":"can_use_tool","tool_name":"Bash","input":{"command":"echo hi"}}}"#);
        let AgentEvent::PermissionRequest { tool_call, .. } = &events[0] else {
            panic!("expected a permission request, got {events:?}");
        };
        assert_eq!(tool_call.id, "r1");
    }

    /// This bridge answers what it understands: another control subtype is not a permission ask,
    /// so it becomes no event and no outstanding request.
    #[test]
    fn a_control_request_of_another_subtype_is_not_a_permission_ask() {
        let line = r#"{"type":"control_request","request_id":"r1",
            "request":{"subtype":"initialize"}}"#;
        let value: Value = serde_json::from_str(line).unwrap();
        assert!(Mapper::default().map_event(&value).is_empty());
        assert!(permission_ask(&value).is_none());
    }

    /// The option ids the event offers are exactly the ones the answer path reads back.
    #[test]
    fn answering_maps_an_option_id_onto_a_behavior() {
        let suggestion = json!({"type":"setMode","mode":"acceptEdits","destination":"session"});
        let suggestions = [suggestion.clone()];
        assert_eq!(answer("allow", &suggestions), ("allow", None));
        assert_eq!(answer("deny", &suggestions), ("deny", None));
        // An "always" carries the suggestion back, which is what makes the choice stick.
        assert_eq!(
            answer("allow_always:0", &suggestions),
            ("allow", Some(json!([suggestion])))
        );
        // Nothing recognisable approved this, so it does not run.
        assert_eq!(answer("allow_always:7", &suggestions), ("deny", None));
        assert_eq!(answer("whatever", &suggestions), ("deny", None));
    }

    /// The plain allow line is the shape verified against 2.1.258, `updatedPermissions` absent.
    #[test]
    fn a_control_response_keeps_the_verified_shape() {
        let line = control_response("r1", "allow", json!({}), None);
        assert_eq!(
            line,
            json!({"type":"control_response","response":{"subtype":"success","request_id":"r1",
                "response":{"behavior":"allow","updatedInput":{}}}})
        );
        let always = control_response("r1", "allow", json!({}), Some(json!([{"a":1}])));
        assert_eq!(
            always["response"]["response"]["updatedPermissions"],
            json!([{"a":1}])
        );
    }

    #[test]
    fn an_unknown_event_is_dropped_rather_than_failing() {
        assert!(map(r#"{"type":"some_future_event","foo":"bar"}"#).is_empty());
    }

    /// The exact shape captured live against Claude Code 2.1.x
    /// (`_docs/wip/agent-setup.md` §"What Claude actually puts on the wire").
    #[test]
    fn a_rate_limit_event_carries_both_windows_and_the_status() {
        let events = map(r#"{"type":"rate_limit_event","rate_limit_info":{
                "status":"allowed","resetsAt":1788474600,"rateLimitType":"five_hour",
                "overageStatus":"rejected","overageDisabledReason":"group_zero_credit_limit",
                "isUsingOverage":false,
                "unifiedWindows":{
                    "five_hour":{"utilization":0.07,"resetsAt":1788474600},
                    "seven_day":{"utilization":0.21,"resetsAt":1788796800}
                }},
                "uuid":"c4051794-ee0c-457f-b210-2a0f053efbb9",
                "session_id":"80c71643-7bbf-4b6d-862c-60ba6e3d6910"}"#);
        assert_eq!(
            events,
            vec![AgentEvent::RateLimitUpdate {
                five_hour: Some(RateLimitWindow {
                    utilization_pct: 7,
                    resets_at: 1_788_474_600,
                }),
                seven_day: Some(RateLimitWindow {
                    utilization_pct: 21,
                    resets_at: 1_788_796_800,
                }),
                status: "allowed".to_string(),
                // Defect 8: both were parsed off the wire and thrown away.
                overage_status: Some("rejected".to_string()),
                overage_reason: Some("group_zero_credit_limit".to_string()),
            }]
        );
    }

    /// Live, two-turn diagnostic against a real, persistent, structured
    /// Claude Code session (`claude -p --input-format stream-json`, prompts
    /// on stdin via [`super::write_input`]) — the actual path Ubiq drives, not
    /// the one-shot `claude -p "prompt"` invocation `discover_models_live_*`
    /// checks. Skips gracefully when `claude` is absent, same convention as
    /// that sibling test (`harness/claude.rs`).
    ///
    /// This is a diagnostic, not (mainly) an assertion-first test: it prints
    /// every event so a human/agent can read what the wire actually did. It
    /// does assert the one thing this bug report needs a permanent regression
    /// guard for — a user-typed turn is echoed back as `UserMessageChunk`.
    #[test]
    fn live_two_turn_structured_session_when_claude_available() {
        use crate::harness::{Claude, FsTemplateStore, Harness};
        use crate::provision::provision;
        use crate::spec::{IoModes, RunSpec};
        use std::process::{Command, Stdio};

        let has_claude = Command::new("claude")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !has_claude {
            eprintln!("skipping: `claude` not on PATH");
            return;
        }

        let cwd = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), cwd.path().to_path_buf());
        spec.io = IoModes::Structured;

        let claude = Claude::new();
        let tmpl_dir = tempfile::TempDir::new().unwrap();
        let templates = FsTemplateStore::new(tmpl_dir.path());
        let provisioned = provision(&claude, &spec, &templates).expect("provision");
        let mut bridge = claude
            .structured_bridge(&provisioned, cwd.path())
            .expect("structured_bridge");

        let mut turn = |n: u32, prompt: &str| -> Vec<AgentEvent> {
            bridge
                .send(AgentInput::prompt(prompt))
                .unwrap_or_else(|e| panic!("turn {n} send: {e}"));
            let mut events = Vec::new();
            loop {
                match bridge.next_event() {
                    Ok(Some(ev)) => {
                        eprintln!("turn {n} event: {ev:?}");
                        let ended = matches!(ev, AgentEvent::TurnEnded { .. });
                        events.push(ev);
                        if ended {
                            break;
                        }
                    }
                    Ok(None) => {
                        eprintln!("turn {n}: stdout closed before TurnEnded");
                        break;
                    }
                    Err(e) => panic!("turn {n} next_event: {e}"),
                }
            }
            events
        };

        let first = turn(1, "Reply with exactly the word BANANA and nothing else.");
        let second = turn(2, "Now reply with exactly the word MANGO and nothing else.");

        for (n, events) in [(1, &first), (2, &second)] {
            let has_user_chunk = events
                .iter()
                .any(|e| matches!(e, AgentEvent::UserMessageChunk { .. }));
            assert!(
                has_user_chunk,
                "turn {n}: expected a UserMessageChunk echoing the sent prompt, got: {events:?}"
            );
        }
    }
}
