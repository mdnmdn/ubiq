//! Codex `app-server`'s JSON-RPC 2.0 bridge — the [`super::IoBridge`]
//! implementation for `codex app-server --listen stdio://`.
//!
//! Speaks the wire protocol documented in `_docs/harness/codex.md`
//! §"Orchestration / headless invocation": newline-delimited JSON-RPC 2.0 on
//! stdin/stdout, one object per line, both directions. Unlike Claude Code's
//! `stream-json` (a flat stream of self-describing events), this is a real
//! RPC: every request `am` sends carries an `id` and gets exactly one
//! matching response, while the server independently pushes notifications
//! (no `id`) and its own approval *requests* (an `id` **and** a `method`,
//! expecting a response from us) at any time, interleaved.
//!
//! This is **core** (always compiled, no feature gate): only `std::process`,
//! `std::sync`, `std::thread`, `std::collections::HashMap`, `serde_json` and
//! `tracing` are used, matching [`super::jsonl`]'s discipline.
//!
//! ## Design
//!
//! [`CodexBridge::new`] takes ownership of a spawned [`std::process::Child`]
//! (from [`super::spawn_piped`]), splits off stdin/stdout, and spawns a
//! dedicated **reader thread** that owns stdout for the bridge's whole
//! lifetime — the same reason as [`super::jsonl`]: the reader must always be
//! draining stdout so a write on [`CodexBridge::send`] (or a blocking
//! request during the handshake) never stalls behind a full pipe buffer.
//!
//! stdin, the pending-request map and the request-id counter are each shared
//! (`Arc<Mutex<..>>` / `Arc<AtomicI64>`) because *four* things write to or
//! allocate from them: the handshake requests in [`CodexBridge::new`],
//! [`CodexBridge::send`]'s `turn/start`, the reader thread's auto-accept
//! responses to server→client approval requests, and any [`CodexInputSink`]
//! handed out through [`IoBridge::input`]. `thread_id` is shared the same way
//! via `Arc<OnceLock<String>>`: it is written exactly once, at the end of the
//! handshake, and every later reader (including a sink on another thread)
//! only ever observes it fully formed or not yet set.
//!
//! ### Request/response correlation
//!
//! Every outbound JSON-RPC *request* (as opposed to notification) is
//! assigned a fresh id from an `AtomicI64` counter and registered in a
//! shared `Arc<Mutex<HashMap<i64, mpsc::Sender<Value>>>>` ("pending map")
//! *before* the line is written, so the reader thread can never observe the
//! response before the sender is registered. The reader thread, on seeing a
//! line shaped like a response (`id` + (`result` or `error`), no `method`),
//! looks up and removes the matching entry and forwards the whole response
//! object down that channel. The caller blocks on `recv_timeout` — never a
//! bare `recv` — so a misbehaving or silent server can never hang the
//! bridge; a timeout removes the (now-stale) pending entry and returns an
//! error.
//!
//! ### Never hanging
//!
//! Three independent guards keep this bridge from ever blocking forever:
//! 1. Every blocking wait on a response (`initialize`, `thread/start`,
//!    `turn/start`'s ack) uses `recv_timeout(REQUEST_TIMEOUT)`.
//! 2. [`CodexBridge::next_event`] blocks on a plain `recv()`, but that
//!    channel's only sender-holders are the reader thread and (briefly)
//!    `new()` — once the reader thread exits (stdout EOF, a channel
//!    disconnect, or a poisoned lock) the channel closes and `recv()`
//!    returns `Err`, mapped to `Ok(None)`.
//! 3. [`Drop`] closes stdin, bounds-waits for the child to exit, then kills
//!    it and joins the reader thread — mirroring
//!    [`super::jsonl::JsonlBridge`]'s teardown.
//!
//! ## Logging
//!
//! Every raw line, in either direction, is a `trace!`; every mapped event is
//! a `debug!`. Raw frames carry prompts and file contents, which is why they
//! sit a level below everything else: an embedder's default filter collects
//! `debug` and leaves them out until someone asks for them by name.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{
    AgentEvent, AgentInput, AgentInputSink, Content, IoBridge, PermissionKind, PermissionOption,
    StopReason, ToolCall, ToolCallUpdate, ToolContent, ToolKind, ToolLocation, ToolStatus,
};

/// How long a blocking request (`initialize`, `thread/start`, `turn/start`'s
/// ack) waits for its matching response before giving up. Bounds every
/// synchronous RPC round trip so a silent/misbehaving app-server can never
/// hang the bridge.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// How long [`Drop`] waits for the child to exit after closing stdin before
/// killing it. Mirrors `_docs/harness/codex.md` §"Process lifecycle": "close
/// stdin … wait ~10s for the reader to drain … Wait up to ~10s more … SIGKILL".
/// We collapse the two ~10s waits into one bounded `try_wait` loop.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// Pending outbound requests awaiting a response, keyed by the `id` we sent.
/// Shared between the bridge (registers before writing), the reader thread
/// (delivers + removes on a matching response line), and any
/// [`CodexInputSink`].
type PendingMap = Arc<Mutex<HashMap<i64, mpsc::Sender<Value>>>>;

/// A live bridge to a `codex app-server --listen stdio://` process speaking
/// JSON-RPC 2.0 on stdin/stdout.
pub struct CodexBridge {
    child: Child,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    events: mpsc::Receiver<AgentEvent>,
    reader: Option<std::thread::JoinHandle<()>>,
    pending: PendingMap,
    next_id: Arc<AtomicI64>,
    /// The `thread.id` captured from `thread/start`'s response, echoed into
    /// every subsequent `turn/start`. Set exactly once, at the end of the
    /// handshake.
    thread_id: Arc<OnceLock<String>>,
}

/// The detached input side of a [`CodexBridge`], for a caller pumping events
/// on one thread and prompting from another. See [`AgentInputSink`].
pub struct CodexInputSink {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    pending: PendingMap,
    next_id: Arc<AtomicI64>,
    thread_id: Arc<OnceLock<String>>,
}

impl AgentInputSink for CodexInputSink {
    fn send(&self, input: AgentInput) -> crate::Result<()> {
        let thread_id = self.thread_id.get().cloned().unwrap_or_default();
        write_input(&self.stdin, &self.pending, &self.next_id, &thread_id, input)
    }
}

impl CodexBridge {
    /// Wrap an already-spawned `codex app-server` child (piped stdin/stdout,
    /// e.g. from [`super::spawn_piped`]) as a [`CodexBridge`], running the
    /// full handshake synchronously: `initialize` → `initialized` →
    /// `thread/start`.
    ///
    /// `cwd` is sent as `thread/start`'s `cwd` param.
    ///
    /// Errors (without hanging — every step is timeout-bounded) if:
    /// - `child`'s stdin/stdout aren't piped (a programmer error —
    ///   [`super::spawn_piped`] always pipes both);
    /// - any handshake request times out or the server responds with a
    ///   JSON-RPC `error`;
    /// - `thread/start`'s response is missing `thread.id`.
    ///
    /// On any handshake error, the partially-built bridge (and its reader
    /// thread + child process) is torn down via [`Drop`] as the function
    /// returns — nothing is leaked.
    pub fn new(mut child: Child, cwd: &Path) -> crate::Result<Self> {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("child stdin is not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("child stdout is not piped"))?;

        let stdin = Arc::new(Mutex::new(Some(stdin)));
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = mpsc::channel();

        let reader_stdin = Arc::clone(&stdin);
        let reader_pending = Arc::clone(&pending);
        let reader_tx = tx.clone();
        let reader =
            std::thread::spawn(move || read_loop(stdout, reader_stdin, reader_pending, reader_tx));

        let mut bridge = Self {
            child,
            stdin,
            events: rx,
            reader: Some(reader),
            pending,
            next_id: Arc::new(AtomicI64::new(1)),
            thread_id: Arc::new(OnceLock::new()),
        };

        bridge.handshake(cwd, &tx)?;

        Ok(bridge)
    }

    /// `initialize` → `initialized` → `thread/start`, capturing `thread.id`
    /// and emitting [`AgentEvent::SessionStarted`] onto `tx` once it's known.
    fn handshake(&mut self, cwd: &Path, tx: &mpsc::Sender<AgentEvent>) -> crate::Result<()> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "agent-manager",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {"experimentalApi": true},
            }),
        )?;

        self.notify("initialized", json!({}))?;

        let thread_resp =
            self.request("thread/start", json!({"cwd": cwd.display().to_string()}))?;
        let thread_id = thread_resp
            .get("thread")
            .and_then(|t| t.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("thread/start response missing thread.id"))?
            .to_string();
        // Set once, here; every later reader (`send`, a `CodexInputSink`)
        // only ever sees it fully formed.
        let _ = self.thread_id.set(thread_id.clone());

        // `thread/start`'s response documents only `thread.id`
        // (`_docs/harness/codex.md` §"Output stream protocol") — Codex never
        // reports back the model, mode, or tool/agent list it's running
        // with, so the rest of the event is empty rather than guessed.
        let ev = AgentEvent::SessionStarted {
            session_id: Some(thread_id),
            model: None,
            mode: None,
            tools: Vec::new(),
            agents: Vec::new(),
        };
        tracing::debug!(event = ?ev, "codex event");
        // Best-effort: if nobody's listening yet (shouldn't happen — `rx` is
        // held by the not-yet-returned bridge), just drop the event.
        let _ = tx.send(ev);

        Ok(())
    }

    /// Send a JSON-RPC *request* (`method` + `params`, with a fresh `id`)
    /// and block (with [`REQUEST_TIMEOUT`]) for its matching response.
    fn request(&self, method: &str, params: Value) -> crate::Result<Value> {
        rpc_request(&self.stdin, &self.pending, &self.next_id, method, params)
    }

    /// Send a JSON-RPC *notification* (`method` + `params`, no `id`) —
    /// fire-and-forget, no response expected.
    fn notify(&self, method: &str, params: Value) -> crate::Result<()> {
        rpc_notify(&self.stdin, method, params)
    }
}

impl IoBridge for CodexBridge {
    fn send(&mut self, input: AgentInput) -> crate::Result<()> {
        let thread_id = self.thread_id.get().cloned().unwrap_or_default();
        write_input(&self.stdin, &self.pending, &self.next_id, &thread_id, input)
    }

    fn next_event(&mut self) -> crate::Result<Option<AgentEvent>> {
        match self.events.recv() {
            Ok(ev) => Ok(Some(ev)),
            // Sender dropped == reader thread exited == stdout hit EOF (or a
            // disconnect/lock failure it treated the same way).
            Err(mpsc::RecvError) => Ok(None),
        }
    }

    fn input(&self) -> Option<Arc<dyn AgentInputSink>> {
        Some(Arc::new(CodexInputSink {
            stdin: Arc::clone(&self.stdin),
            pending: Arc::clone(&self.pending),
            next_id: Arc::clone(&self.next_id),
            thread_id: Arc::clone(&self.thread_id),
        }))
    }
}

impl Drop for CodexBridge {
    fn drop(&mut self) {
        // Close stdin first (best-effort "please stop" signal), then give
        // the child a bounded window to drain/exit before killing it.
        if let Ok(mut guard) = self.stdin.lock() {
            *guard = None;
        }

        let deadline = Instant::now() + DRAIN_TIMEOUT;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        // `_docs/harness/codex.md` §"Process lifecycle" calls
                        // for SIGKILLing the whole process GROUP (negative
                        // PID) so any grandchildren die too. Doing that needs
                        // a `setpgid`/`killpg` syscall wrapper, and this
                        // crate is `#![forbid(unsafe_code)]` with no
                        // libc/nix dependency to provide one.
                        // NOTE: full process-group teardown deferred (needs
                        // a syscall wrapper; unsafe-free constraint) — this
                        // kills only the direct child.
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

/// Send a JSON-RPC *request* and block (with [`REQUEST_TIMEOUT`]) for its
/// matching response. Free function so [`CodexBridge::request`] and (via
/// [`write_input`]) a detached [`CodexInputSink`] share one implementation.
///
/// Registers the id in `pending` before writing the line, so the reader
/// thread can never observe (and drop) the response before this call is
/// ready for it. Returns the response's `result` field (or an error if the
/// response carried a JSON-RPC `error`, or if the wait timed out / the
/// channel disconnected).
fn rpc_request(
    stdin: &Arc<Mutex<Option<ChildStdin>>>,
    pending: &PendingMap,
    next_id: &AtomicI64,
    method: &str,
    params: Value,
) -> crate::Result<Value> {
    let id = next_id.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = mpsc::channel();
    {
        let mut pending = pending
            .lock()
            .map_err(|_| anyhow::anyhow!("codex bridge pending-map lock poisoned"))?;
        pending.insert(id, tx);
    }

    let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    if let Err(err) = write_line(stdin, &line) {
        if let Ok(mut pending) = pending.lock() {
            pending.remove(&id);
        }
        return Err(err);
    }

    match rx.recv_timeout(REQUEST_TIMEOUT) {
        Ok(response) => {
            if let Some(error) = response.get("error") {
                anyhow::bail!("codex app-server returned an error for `{method}`: {error}");
            }
            Ok(response.get("result").cloned().unwrap_or(Value::Null))
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            if let Ok(mut pending) = pending.lock() {
                pending.remove(&id);
            }
            anyhow::bail!(
                "timed out after {:?} waiting for a response to `{method}`",
                REQUEST_TIMEOUT
            )
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            anyhow::bail!(
                "codex app-server's stdout closed while waiting for a response to `{method}`"
            )
        }
    }
}

/// Send a JSON-RPC *notification* — fire-and-forget, no response expected.
/// Free function for the same reason as [`rpc_request`].
fn rpc_notify(
    stdin: &Arc<Mutex<Option<ChildStdin>>>,
    method: &str,
    params: Value,
) -> crate::Result<()> {
    let line = json!({"jsonrpc": "2.0", "method": method, "params": params});
    write_line(stdin, &line)
}

/// Turn one [`AgentInput`] into the JSON-RPC call(s) it implies, and send
/// them.
///
/// Shared by [`CodexBridge::send`] and [`CodexInputSink`] so the two cannot
/// drift apart — a prompt sent from a pump thread has to reach the child in
/// exactly the same shape as one sent from the owner's.
fn write_input(
    stdin: &Arc<Mutex<Option<ChildStdin>>>,
    pending: &PendingMap,
    next_id: &AtomicI64,
    thread_id: &str,
    input: AgentInput,
) -> crate::Result<()> {
    match input {
        AgentInput::Prompt { .. } => {
            // Block only on `turn/start`'s ack (which carries `turn.id`),
            // NOT on turn completion — that arrives later as
            // `turn/completed` / `thread/status/changed` notifications,
            // read back via `next_event`.
            let text = input.prompt_text().unwrap_or_default();
            rpc_request(
                stdin,
                pending,
                next_id,
                "turn/start",
                json!({
                    "threadId": thread_id,
                    "input": [{"type": "text", "text": text}],
                }),
            )?;
            Ok(())
        }
        AgentInput::AnswerPermission { .. } => {
            // A documented no-op: every server→client approval request
            // (`item/commandExecution/requestApproval`,
            // `item/fileChange/requestApproval`,
            // `item/permissions/requestApproval`,
            // `mcpServer/elicitation/request`) is already auto-accepted by
            // the reader thread the moment it's scanned off stdout (see
            // `read_loop` / `approval_response`), matching
            // `_docs/harness/codex.md` §"Tool approval in headless mode".
            // `am` doesn't track pending approval ids to answer out-of-band,
            // so a caller-issued `AnswerPermission` has nothing left to do.
            Ok(())
        }
        AgentInput::Cancel => {
            // Best-effort, matching `_docs/harness/codex.md`
            // §"Process lifecycle": "close stdin to signal the app-server
            // to stop". Codex's JSON-RPC surface has no documented
            // `turn/cancel`-style request, so closing stdin (same mechanism
            // [`Drop`] uses) is the only signal we send; the reader thread
            // keeps draining stdout until the process actually exits.
            if let Ok(mut guard) = stdin.lock() {
                *guard = None;
            }
            Ok(())
        }
        AgentInput::SetConfigOption { config_id, .. } => Err(anyhow::anyhow!(
            "codex's app-server bridge cannot change '{config_id}' mid-session"
        )),
    }
}

/// The reader thread body: scan `stdout` line-by-line (newline-delimited
/// JSON-RPC), and route each parsed line by shape:
/// - a **response** to one of our requests (`id` + (`result` or `error`),
///   no `method`) → deliver to the waiting [`rpc_request`] call via
///   `pending`;
/// - a **server→client request** (`id` **and** `method`) → auto-accept: emit
///   an (optional) [`AgentEvent::PermissionRequest`] for visibility, then
///   write a matching JSON-RPC response with the *same* `id` back on `stdin`;
/// - a **notification** (`method`, no `id`) → [`map_notification`] to zero
///   or more [`AgentEvent`]s.
///
/// Returns (dropping `tx`, which closes the event channel) on stdout EOF, a
/// channel disconnect (nobody left to receive), or a poisoned lock.
fn read_loop(
    stdout: ChildStdout,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    pending: PendingMap,
    tx: mpsc::Sender<AgentEvent>,
) {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        tracing::trace!(direction = "in", frame = %line, "codex jsonrpc");
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            // Not a recognized JSON line — ignore rather than error.
            continue;
        };

        let id = value.get("id").cloned();
        let method = value.get("method").and_then(Value::as_str);
        let has_result_or_error = value.get("result").is_some() || value.get("error").is_some();

        match (id, method) {
            // A response to one of OUR requests.
            (Some(id_val), None) if has_result_or_error => {
                let Some(id_num) = id_val.as_i64() else {
                    continue;
                };
                let sender = match pending.lock() {
                    Ok(mut guard) => guard.remove(&id_num),
                    Err(_) => return,
                };
                if let Some(sender) = sender {
                    // If nobody's listening anymore (the requester timed out
                    // and gave up), silently drop — nothing to do.
                    let _ = sender.send(value);
                }
            }
            // A server→client request: has BOTH an id and a method, and
            // expects a response.
            (Some(id_val), Some(method_name)) => {
                for ev in approval_event(method_name, &value) {
                    tracing::debug!(event = ?ev, "codex event");
                    if tx.send(ev).is_err() {
                        return;
                    }
                }
                if let Some(response) = approval_response(id_val, method_name)
                    && write_line(&stdin, &response).is_err()
                {
                    return;
                }
            }
            // A notification: method, no id.
            (None, Some(_)) => {
                for ev in map_notification(&value) {
                    tracing::debug!(event = ?ev, "codex event");
                    if tx.send(ev).is_err() {
                        return;
                    }
                }
            }
            _ => {}
        }
    }
}

/// `true` if `method` is one of the four server→client approval request
/// kinds documented in `_docs/harness/codex.md` §"Tool approval in headless
/// mode".
fn is_approval_method(method: &str) -> bool {
    matches!(
        method,
        "item/commandExecution/requestApproval"
            | "execCommandApproval"
            | "item/fileChange/requestApproval"
            | "applyPatchApproval"
            | "item/permissions/requestApproval"
            | "mcpServer/elicitation/request"
    )
}

/// What kind of tool an approval method is asking about — `Execute` for a
/// command, `Edit` for a file change, `Other` for the two kinds
/// (`permissions`, `elicitation`) that aren't tied to one tool call.
fn approval_kind(method: &str) -> ToolKind {
    match method {
        "item/commandExecution/requestApproval" | "execCommandApproval" => ToolKind::Execute,
        "item/fileChange/requestApproval" | "applyPatchApproval" => ToolKind::Edit,
        _ => ToolKind::Other,
    }
}

/// A human label for an approval method, since `_docs/harness/codex.md`
/// §"Tool approval in headless mode" pins down the response shape per method
/// but not a request param shape a title could be built from.
fn approval_title(method: &str) -> &'static str {
    match method {
        "item/commandExecution/requestApproval" | "execCommandApproval" => "Run a command",
        "item/fileChange/requestApproval" | "applyPatchApproval" => "Apply a file change",
        "item/permissions/requestApproval" => "Grant permissions",
        "mcpServer/elicitation/request" => "MCP elicitation",
        _ => "Approval request",
    }
}

/// The two options every Codex approval offers. `am` never actually routes
/// an [`AgentInput::AnswerPermission`] back into one of these — the reader
/// thread has already auto-accepted the request (see [`approval_response`])
/// by the time a caller could answer — so these exist for a consumer to draw
/// the dialog it briefly had a say in, not because a decision made here
/// changes anything.
fn approval_options() -> Vec<PermissionOption> {
    vec![
        PermissionOption {
            option_id: "accept".to_string(),
            name: "Allow".to_string(),
            kind: PermissionKind::AllowOnce,
        },
        PermissionOption {
            option_id: "decline".to_string(),
            name: "Deny".to_string(),
            kind: PermissionKind::RejectOnce,
        },
    ]
}

/// Build the (optional, for-visibility) [`AgentEvent::PermissionRequest`] for
/// a server→client approval request, before it's auto-accepted.
fn approval_event(method: &str, value: &Value) -> Vec<AgentEvent> {
    if !is_approval_method(method) {
        return Vec::new();
    }
    let request_id = value
        .get("id")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();

    vec![AgentEvent::PermissionRequest {
        request_id: request_id.clone(),
        tool_call: ToolCallUpdate {
            id: request_id,
            title: Some(approval_title(method).to_string()),
            kind: Some(approval_kind(method)),
            status: Some(ToolStatus::Pending),
            // The request's own param shape isn't pinned down beyond "what
            // it's approving", so the raw params are kept here for a
            // consumer that wants to dig further rather than guessed at.
            raw_output: value.get("params").cloned(),
            ..ToolCallUpdate::default()
        },
        options: approval_options(),
    }]
}

/// Build the JSON-RPC response auto-accepting a server→client approval
/// request, per `_docs/harness/codex.md` §"Tool approval in headless mode"'s
/// mapping table. Echoes `id` back unchanged (works for either a numeric or
/// string id, whichever the server used). `None` if `method` isn't a
/// recognized approval kind (nothing to answer).
fn approval_response(id: Value, method: &str) -> Option<Value> {
    let result = match method {
        "item/commandExecution/requestApproval" | "execCommandApproval" => {
            json!({"decision": "accept"})
        }
        "item/fileChange/requestApproval" | "applyPatchApproval" => {
            json!({"decision": "accept"})
        }
        // Grant network + fileSystem, scoped to the current turn. The exact
        // response shape isn't pinned down by codex.md beyond "grant network
        // + fileSystem, scoped to turn"; this is a reasonable encoding of
        // that grant. See the module-level doc comment / task report for
        // this ambiguity.
        "item/permissions/requestApproval" => json!({
            "decision": "accept",
            "scope": "turn",
            "permissions": {"network": true, "fileSystem": true},
        }),
        "mcpServer/elicitation/request" => json!({"action": "accept", "content": Value::Null}),
        _ => return None,
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// Map one parsed JSON-RPC *notification* (`method` + `params`, no `id`) to
/// zero or more [`AgentEvent`]s, supporting both of codex's notification
/// dialects (`_docs/harness/codex.md` §"Output stream protocol"):
/// - **Legacy**: a single `codex/event` method wrapping `params.msg.type`.
/// - **v2/raw**: discrete method names (`turn/started`, `item/started`,
///   `item/completed`, `turn/completed`, `thread/status/changed`, `error`).
///
/// Pure (no I/O) so it's unit-testable directly. Unknown methods/subtypes
/// are ignored.
pub(crate) fn map_notification(value: &Value) -> Vec<AgentEvent> {
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return Vec::new();
    };
    let params = value.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "codex/event" => map_legacy_event(&params),
        // No direct AgentEvent equivalent for "a turn began" — the eventual
        // completion (`turn/completed` / idle `thread/status/changed`) is
        // what matters to a consumer.
        "turn/started" => Vec::new(),
        "item/started" => map_item(&params, true),
        "item/completed" => map_item(&params, false),
        "turn/completed" => map_turn_completed(&params),
        "thread/status/changed" => map_thread_status_changed(&params),
        "error" => map_error(&params),
        _ => Vec::new(),
    }
}

/// Codex's own item-type/verb vocabulary (`exec_command` in the legacy
/// dialect, `commandExecution` in v2, `fileChange` in v2) onto the ten kinds
/// a consumer draws. Unmapped values fall back to [`ToolKind::Other`] rather
/// than a guess.
fn tool_kind(verb: &str) -> ToolKind {
    match verb {
        "exec_command" | "commandExecution" => ToolKind::Execute,
        "fileChange" => ToolKind::Edit,
        _ => ToolKind::Other,
    }
}

/// A human-readable target for a tool call's title: the command line, if
/// `input` is one — Codex sends it as either a bare string or an argv array.
/// `None` for anything else (a `fileChange`'s `changes`, whose shape isn't
/// documented) rather than a guessed field name.
fn command_target(input: &Value) -> Option<String> {
    match input {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            let parts: Vec<&str> = items.iter().filter_map(Value::as_str).collect();
            (!parts.is_empty()).then(|| parts.join(" "))
        }
        _ => None,
    }
}

/// A tool call starting, in the shape a transcript draws: a verb and,
/// where `input` is a command, the command line itself.
fn command_tool_call(id: Option<&str>, verb: &str, input: Option<&Value>) -> ToolCall {
    let target = input.and_then(command_target);
    let title = match &target {
        Some(target) => format!("{verb} {target}"),
        None => verb.to_string(),
    };

    let mut call = ToolCall::new(id.unwrap_or(verb), title);
    call.kind = tool_kind(verb);
    call.status = ToolStatus::InProgress;
    call.raw_input = input.cloned();
    call
}

/// Best-effort file locations out of a `fileChange` item's `changes` field.
/// `_docs/harness/codex.md` documents the field's existence, not its shape,
/// so this only claims a location where a `path` string is actually present
/// — either on `changes` itself or on each entry of it, if it's an array —
/// and yields nothing rather than guessing at other shapes.
fn locations_from_changes(changes: Option<&Value>) -> Vec<ToolLocation> {
    let Some(changes) = changes else {
        return Vec::new();
    };
    let entries: Vec<&Value> = match changes {
        Value::Array(items) => items.iter().collect(),
        Value::Object(_) => vec![changes],
        _ => Vec::new(),
    };
    entries
        .into_iter()
        .filter_map(|entry| entry.get("path").and_then(Value::as_str))
        .map(|path| ToolLocation {
            path: path.to_string(),
            line: None,
        })
        .collect()
}

/// A tool call finishing. Codex's `exec_command_end` / `item/completed`
/// items carry no documented success/failure field (unlike Claude Code's
/// `is_error`), so a result is always `Completed` here — a real failure
/// signal, if Codex ever documents one, belongs in this one place.
fn tool_result_update(id: &str, output: Option<&Value>) -> ToolCallUpdate {
    let content = output.and_then(Value::as_str).map(|text| {
        vec![ToolContent::Content {
            content: Content::text(text),
        }]
    });
    ToolCallUpdate {
        id: id.to_string(),
        status: Some(ToolStatus::Completed),
        content,
        raw_output: output.cloned(),
        ..ToolCallUpdate::default()
    }
}

/// Map a legacy `codex/event` notification's `params.msg` per the "Legacy"
/// column of codex.md's canonical category mapping table.
fn map_legacy_event(params: &Value) -> Vec<AgentEvent> {
    let Some(msg) = params.get("msg") else {
        return Vec::new();
    };
    let Some(msg_type) = msg.get("type").and_then(Value::as_str) else {
        return Vec::new();
    };

    match msg_type {
        "agent_message" => {
            let text = msg
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| msg.get("text").and_then(Value::as_str))
                .unwrap_or_default();
            // The legacy dialect carries no message id to group chunks by.
            vec![AgentEvent::AgentMessageChunk {
                content: Content::text(text),
                message_id: None,
            }]
        }
        "exec_command_begin" => {
            let id = msg.get("call_id").and_then(Value::as_str);
            vec![AgentEvent::ToolCall {
                call: command_tool_call(id, "exec_command", msg.get("command")),
            }]
        }
        "exec_command_end" => {
            let id = msg
                .get("call_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let output = msg.get("output").or_else(|| msg.get("stdout"));
            vec![AgentEvent::ToolCallUpdate {
                update: tool_result_update(id, output),
            }]
        }
        // Optional per the task spec; carries no session id (or model, mode,
        // tools, agents) at the legacy layer.
        "task_started" => vec![AgentEvent::SessionStarted {
            session_id: None,
            model: None,
            mode: None,
            tools: Vec::new(),
            agents: Vec::new(),
        }],
        "task_complete" => vec![AgentEvent::TurnEnded {
            stop_reason: StopReason::EndTurn,
            error: None,
        }],
        // Distinct from a generic failure: the vocabulary has a dedicated
        // `Cancelled` stop reason and "aborted" is exactly that, not the
        // run breaking.
        "turn_aborted" => vec![AgentEvent::TurnEnded {
            stop_reason: StopReason::Cancelled,
            error: msg
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_string),
        }],
        _ => Vec::new(),
    }
}

/// Map a v2 `item/started` (`started: true`) or `item/completed`
/// (`started: false`) notification's `params.item`, keyed by `itemType`.
fn map_item(params: &Value, started: bool) -> Vec<AgentEvent> {
    let Some(item) = params.get("item") else {
        return Vec::new();
    };
    let item_type = item
        .get("itemType")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let id = item.get("id").and_then(Value::as_str);

    match (item_type, started) {
        ("commandExecution", true) => vec![AgentEvent::ToolCall {
            call: command_tool_call(id, "commandExecution", item.get("command")),
        }],
        ("commandExecution", false) => {
            let output = item.get("output").or_else(|| item.get("result"));
            vec![AgentEvent::ToolCallUpdate {
                update: tool_result_update(id.unwrap_or_default(), output),
            }]
        }
        ("fileChange", true) => {
            let mut call = command_tool_call(id, "fileChange", item.get("changes"));
            call.locations = locations_from_changes(item.get("changes"));
            vec![AgentEvent::ToolCall { call }]
        }
        ("fileChange", false) => {
            let output = item.get("result");
            vec![AgentEvent::ToolCallUpdate {
                update: tool_result_update(id.unwrap_or_default(), output),
            }]
        }
        // Per codex.md's mapping table, only the *completed* agentMessage
        // maps to assistant text; there's no meaningful "started" event for
        // it (falls through to the wildcard below).
        ("agentMessage", false) => {
            let text = item
                .get("text")
                .and_then(Value::as_str)
                .or_else(|| item.get("content").and_then(Value::as_str))
                .unwrap_or_default();
            vec![AgentEvent::AgentMessageChunk {
                content: Content::text(text),
                message_id: id.map(str::to_string),
            }]
        }
        _ => Vec::new(),
    }
}

/// Map a v2 `turn/completed` notification to the turn's end.
///
/// Codex reports token counts here (under `turn.usage` / `usage` /
/// `token_usage` / `tokens`) but never a context window size
/// (`_docs/harness/codex.md` has no `contextWindow`-shaped field anywhere in
/// the headless surface), so there is no denominator for a
/// [`AgentEvent::UsageUpdate`] ring — and a ratio with an invented one is
/// worse than no ratio. The counts are dropped rather than reported half.
fn map_turn_completed(_params: &Value) -> Vec<AgentEvent> {
    vec![AgentEvent::TurnEnded {
        stop_reason: StopReason::EndTurn,
        error: None,
    }]
}

/// Map a v2 `thread/status/changed` notification: `status.type == "idle"`
/// is a terminal success (mirrors `turn/completed`); anything else is
/// ignored (not a documented terminal state).
fn map_thread_status_changed(params: &Value) -> Vec<AgentEvent> {
    let is_idle = params
        .get("status")
        .and_then(|s| s.get("type"))
        .and_then(Value::as_str)
        == Some("idle");
    if is_idle {
        vec![AgentEvent::TurnEnded {
            stop_reason: StopReason::EndTurn,
            error: None,
        }]
    } else {
        Vec::new()
    }
}

/// Map a v2 `error` notification: terminal only when `willRetry` is
/// `false` (absent defaults to non-retrying, i.e. terminal) — a retrying
/// error isn't the end of the run, so it's ignored here.
fn map_error(params: &Value) -> Vec<AgentEvent> {
    let will_retry = params
        .get("willRetry")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if will_retry {
        return Vec::new();
    }
    let error = params
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| params.get("error").and_then(Value::as_str))
        .map(str::to_string);
    vec![AgentEvent::TurnEnded {
        stop_reason: StopReason::Failed,
        error,
    }]
}

/// Serialize `value` as one newline-delimited JSON-RPC line and write it to
/// the shared stdin, under the shared lock. A `None` stdin (closed, e.g.
/// after [`AgentInput::Cancel`] or during [`Drop`]) is a silent no-op
/// rather than an error — the process is already being told to stop.
fn write_line(stdin: &Arc<Mutex<Option<ChildStdin>>>, value: &Value) -> crate::Result<()> {
    let mut guard = stdin
        .lock()
        .map_err(|_| anyhow::anyhow!("codex bridge stdin lock poisoned"))?;
    if let Some(stdin) = guard.as_mut() {
        tracing::trace!(direction = "out", frame = %value, "codex jsonrpc");
        writeln!(stdin, "{value}")?;
        stdin.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(json: &str) -> Vec<AgentEvent> {
        let value: Value = serde_json::from_str(json).unwrap();
        map_notification(&value)
    }

    #[test]
    fn map_notification_legacy_agent_message_is_assistant_text() {
        let events = map(r#"{"jsonrpc":"2.0","method":"codex/event",
                "params":{"msg":{"type":"agent_message","message":"hi from legacy"}}}"#);
        assert_eq!(
            events,
            vec![AgentEvent::AgentMessageChunk {
                content: Content::text("hi from legacy"),
                message_id: None,
            }]
        );
    }

    #[test]
    fn map_notification_legacy_exec_command_begin_and_end() {
        let begin: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"codex/event",
                "params":{"msg":{"type":"exec_command_begin","call_id":"c1","command":["ls"]}}}"#,
        )
        .unwrap();
        let events = map_notification(&begin);
        let AgentEvent::ToolCall { call } = &events[0] else {
            panic!("expected a tool call, got {events:?}");
        };
        assert_eq!(call.id, "c1");
        assert_eq!(call.title, "exec_command ls");
        assert_eq!(call.kind, ToolKind::Execute);
        assert_eq!(call.status, ToolStatus::InProgress);
        assert_eq!(call.raw_input, Some(json!(["ls"])));

        let end: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"codex/event",
                "params":{"msg":{"type":"exec_command_end","call_id":"c1","output":"ok"}}}"#,
        )
        .unwrap();
        let events = map_notification(&end);
        let AgentEvent::ToolCallUpdate { update } = &events[0] else {
            panic!("expected an update, got {events:?}");
        };
        assert_eq!(update.id, "c1");
        assert_eq!(update.status, Some(ToolStatus::Completed));
        assert_eq!(
            update.content,
            Some(vec![ToolContent::Content {
                content: Content::text("ok"),
            }])
        );
        assert_eq!(update.raw_output, Some(json!("ok")));
    }

    #[test]
    fn map_notification_legacy_task_complete_and_turn_aborted() {
        let complete: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"codex/event","params":{"msg":{"type":"task_complete"}}}"#,
        )
        .unwrap();
        assert_eq!(
            map_notification(&complete),
            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::EndTurn,
                error: None,
            }]
        );

        let aborted: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"codex/event",
                "params":{"msg":{"type":"turn_aborted","reason":"cancelled"}}}"#,
        )
        .unwrap();
        assert_eq!(
            map_notification(&aborted),
            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::Cancelled,
                error: Some("cancelled".to_string()),
            }]
        );
    }

    #[test]
    fn map_notification_v2_item_completed_agent_message_is_assistant_text() {
        let v: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"item/completed",
                "params":{"item":{"id":"i1","itemType":"agentMessage","text":"hi from v2"}}}"#,
        )
        .unwrap();
        assert_eq!(
            map_notification(&v),
            vec![AgentEvent::AgentMessageChunk {
                content: Content::text("hi from v2"),
                message_id: Some("i1".to_string()),
            }]
        );
    }

    #[test]
    fn map_notification_v2_item_started_and_completed_command_execution() {
        let started: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"item/started",
                "params":{"item":{"id":"c1","itemType":"commandExecution","command":"ls"}}}"#,
        )
        .unwrap();
        let events = map_notification(&started);
        let AgentEvent::ToolCall { call } = &events[0] else {
            panic!("expected a tool call, got {events:?}");
        };
        assert_eq!(call.id, "c1");
        assert_eq!(call.title, "commandExecution ls");
        assert_eq!(call.kind, ToolKind::Execute);
        assert_eq!(call.status, ToolStatus::InProgress);

        let completed: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"item/completed",
                "params":{"item":{"id":"c1","itemType":"commandExecution","output":"ok"}}}"#,
        )
        .unwrap();
        let events = map_notification(&completed);
        let AgentEvent::ToolCallUpdate { update } = &events[0] else {
            panic!("expected an update, got {events:?}");
        };
        assert_eq!(update.id, "c1");
        assert_eq!(update.status, Some(ToolStatus::Completed));
        assert_eq!(
            update.content,
            Some(vec![ToolContent::Content {
                content: Content::text("ok"),
            }])
        );
    }

    /// `changes` isn't documented beyond its existence, so a `path` on each
    /// entry is picked up when present and nothing is invented when it
    /// isn't.
    #[test]
    fn map_notification_v2_item_started_file_change_locations_when_a_path_is_known() {
        let v: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"item/started",
                "params":{"item":{"id":"f1","itemType":"fileChange",
                  "changes":[{"path":"/tmp/a.rs"}]}}}"#,
        )
        .unwrap();
        let events = map_notification(&v);
        let AgentEvent::ToolCall { call } = &events[0] else {
            panic!("expected a tool call, got {events:?}");
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

    /// Codex reports token counts on `turn/completed` but never a context
    /// window, so the notification still ends the turn — it just never
    /// produces a usage ratio with an invented denominator.
    #[test]
    fn map_notification_v2_turn_completed_drops_usage_without_a_window() {
        let v: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"turn/completed",
                "params":{"turn":{"usage":{"input_tokens":11,"output_tokens":22}}}}"#,
        )
        .unwrap();
        assert_eq!(
            map_notification(&v),
            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::EndTurn,
                error: None,
            }]
        );
    }

    #[test]
    fn map_notification_v2_thread_status_changed_idle_is_terminal() {
        let v: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"thread/status/changed",
                "params":{"status":{"type":"idle"}}}"#,
        )
        .unwrap();
        assert_eq!(
            map_notification(&v),
            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::EndTurn,
                error: None,
            }]
        );

        let busy: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"thread/status/changed",
                "params":{"status":{"type":"working"}}}"#,
        )
        .unwrap();
        assert_eq!(map_notification(&busy), Vec::new());
    }

    #[test]
    fn map_notification_v2_error_terminal_vs_will_retry() {
        let terminal: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"error",
                "params":{"willRetry":false,"message":"boom"}}"#,
        )
        .unwrap();
        assert_eq!(
            map_notification(&terminal),
            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::Failed,
                error: Some("boom".to_string()),
            }]
        );

        let retrying: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"error",
                "params":{"willRetry":true,"message":"transient"}}"#,
        )
        .unwrap();
        assert_eq!(map_notification(&retrying), Vec::new());
    }

    #[test]
    fn map_notification_unknown_method_is_ignored() {
        let v: Value =
            serde_json::from_str(r#"{"jsonrpc":"2.0","method":"something/new","params":{}}"#)
                .unwrap();
        assert_eq!(map_notification(&v), Vec::new());
    }

    #[test]
    fn approval_event_surfaces_as_a_permission_request() {
        let v: Value = serde_json::from_str(
            r#"{"id":"r1","method":"item/commandExecution/requestApproval",
                "params":{"command":"rm -rf /tmp/x"}}"#,
        )
        .unwrap();
        let events = approval_event("item/commandExecution/requestApproval", &v);
        let AgentEvent::PermissionRequest {
            request_id,
            tool_call,
            options,
        } = &events[0]
        else {
            panic!("expected a permission request, got {events:?}");
        };
        assert_eq!(request_id, "r1");
        assert_eq!(tool_call.kind, Some(ToolKind::Execute));
        assert_eq!(tool_call.status, Some(ToolStatus::Pending));
        assert_eq!(options[0].kind, PermissionKind::AllowOnce);
        assert_eq!(options[1].kind, PermissionKind::RejectOnce);
    }

    #[test]
    fn approval_event_ignores_non_approval_methods() {
        let v: Value = serde_json::from_str(r#"{"id":1,"method":"thread/start"}"#).unwrap();
        assert!(approval_event("thread/start", &v).is_empty());
    }

    #[test]
    fn approval_response_covers_all_four_kinds() {
        for method in [
            "item/commandExecution/requestApproval",
            "execCommandApproval",
            "item/fileChange/requestApproval",
            "applyPatchApproval",
        ] {
            let resp = approval_response(json!(1), method).unwrap();
            assert_eq!(resp["result"]["decision"], json!("accept"));
        }

        let perms = approval_response(json!(2), "item/permissions/requestApproval").unwrap();
        assert_eq!(perms["result"]["decision"], json!("accept"));
        assert_eq!(perms["result"]["permissions"]["network"], json!(true));
        assert_eq!(perms["result"]["permissions"]["fileSystem"], json!(true));

        let elicit = approval_response(json!(3), "mcpServer/elicitation/request").unwrap();
        assert_eq!(elicit["result"]["action"], json!("accept"));

        assert!(approval_response(json!(4), "not/an/approval").is_none());
    }
}
