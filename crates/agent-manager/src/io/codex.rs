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
//! Everything the writers and the reader share lives in one cloneable `Wire`: stdin, the
//! pending-request map and id counter, the thread and live-turn ids, the parked server requests
//! (`asks`) and the model/effort picks. Several things write to it — the handshake, a prompt, a
//! cancel, an answer to a parked request, a pick, the reader's immediate refusals — from the
//! owner's thread or any [`CodexInputSink`]. `thread_id` is written exactly once, at the end of the
//! handshake, and every later reader only ever observes it fully formed or not yet set.
//!
//! What a frame maps to is [`Mapper`]'s: the conversation's own thread versus a subagent's, usage
//! against the stated window, the pushed `codex` rate limit. Approvals are never answered here —
//! they are parked for a person (`D209`).
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

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{
    AgentEvent, AgentInput, AgentInputSink, Content, IoBridge, Origin, PermissionKind,
    PermissionOption, PlanEntry, PlanPriority, PlanStatus, Spend, StopReason, ToolCall,
    ToolCallUpdate, ToolContent, ToolKind, ToolLocation, ToolStatus,
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

/// The live turn's id, shared between the writers and the reader — a prompt sent from one thread
/// has to be interruptible (or steerable) from another, and the reader clears it when the
/// conversation's own turn completes.
type TurnSlot = Arc<Mutex<TurnState>>;

/// What the writers and the reader share about the conversation's own turn.
#[derive(Debug, Default)]
struct TurnState {
    /// The live turn's id; `None` between turns.
    id: Option<String>,
    /// Prompts merged into the live turn (a `turn/steer`, or a `turn/start` the server answered as
    /// one). The host counts a turn per prompt and an end per `TurnEnded`, and Codex ends the
    /// merged turn once — so the reader adds one `TurnEnded` per merged prompt when it does.
    merged: u32,
    /// The last turn the reader saw complete: an ack that names it arrived late and must not be
    /// stored as live.
    done: Option<String>,
}

/// A server→client request parked for a human, keyed by the id it is surfaced under.
#[derive(Debug, Clone)]
struct Ask {
    /// The JSON-RPC id to answer, exactly as the server sent it.
    id: Value,
    method: String,
    params: Value,
}

/// Every [`Ask`] still waiting, by the `request_id` its [`AgentEvent::PermissionRequest`] named.
type AskMap = Arc<Mutex<HashMap<String, Ask>>>;

/// What the thread runs with, and what the pickers were drawn from: the model catalogue
/// (`model/list`), the thread's own model and effort, and any picked since — which every later
/// `turn/start` carries.
#[derive(Debug, Default)]
struct Picks {
    models: Vec<Value>,
    model: Option<String>,
    effort: Option<String>,
}

/// How a bridge opens its thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadOpen {
    /// `thread/start`: a fresh thread.
    Start,
    /// `thread/resume { threadId }`: the same thread, carried on.
    Resume(String),
    /// `thread/fork { threadId }`: a new thread holding a copy of that one's history.
    Fork(String),
}

/// Everything the writers and the reader share. Cloned into the reader and into every
/// [`CodexInputSink`], so a prompt, a cancel, an answer and a pick reach the child in one shape
/// whichever thread sends them.
#[derive(Clone)]
struct Wire {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    pending: PendingMap,
    next_id: Arc<AtomicI64>,
    /// The thread `thread/start` (or resume, or fork) returned. Set once, at the end of the
    /// handshake. It is also the *root* the reader filters on: a spawned subagent runs on a thread
    /// of its own whose notifications arrive on this same connection.
    thread_id: Arc<OnceLock<String>>,
    /// The `turn.id` the live turn was acked with — what `turn/interrupt` and `turn/steer` need.
    /// `None` between turns.
    turn_id: TurnSlot,
    asks: AskMap,
    picks: Arc<Mutex<Picks>>,
    /// A way for a writer to say something on the event stream (a re-drawn picker). Emptied by the
    /// reader as it exits, so the channel still closes when the child is gone.
    events: Arc<Mutex<Option<mpsc::Sender<AgentEvent>>>>,
}

impl Wire {
    fn request(&self, method: &str, params: Value) -> crate::Result<Value> {
        rpc_request(&self.stdin, &self.pending, &self.next_id, method, params)
    }

    fn notify(&self, method: &str, params: Value) -> crate::Result<()> {
        rpc_notify(&self.stdin, method, params)
    }

    fn emit(&self, event: AgentEvent) {
        tracing::debug!(event = ?event, "codex event");
        if let Ok(guard) = self.events.lock()
            && let Some(tx) = guard.as_ref()
        {
            let _ = tx.send(event);
        }
    }

    fn thread(&self) -> String {
        self.thread_id.get().cloned().unwrap_or_default()
    }
}

/// A live bridge to a `codex app-server --listen stdio://` process speaking
/// JSON-RPC 2.0 on stdin/stdout.
pub struct CodexBridge {
    child: Child,
    events: mpsc::Receiver<AgentEvent>,
    reader: Option<std::thread::JoinHandle<()>>,
    wire: Wire,
    /// The model the thread was opened with, for the reader to put on every usage report.
    model: Arc<OnceLock<String>>,
}

/// The detached input side of a [`CodexBridge`], for a caller pumping events
/// on one thread and prompting from another. See [`AgentInputSink`].
pub struct CodexInputSink {
    wire: Wire,
}

impl AgentInputSink for CodexInputSink {
    fn send(&self, input: AgentInput) -> crate::Result<()> {
        write_input(&self.wire, input)
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
    pub fn new(child: Child, cwd: &Path) -> crate::Result<Self> {
        Self::open(child, cwd, ThreadOpen::Start)
    }

    /// [`Self::new`], opening the thread the way `open` says: fresh, resumed by id
    /// (`thread/resume`), or forked from one (`thread/fork`).
    pub fn open(mut child: Child, cwd: &Path, open: ThreadOpen) -> crate::Result<Self> {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("child stdin is not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("child stdout is not piped"))?;

        let (tx, rx) = mpsc::channel();
        let wire = Wire {
            stdin: Arc::new(Mutex::new(Some(stdin))),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicI64::new(1)),
            thread_id: Arc::new(OnceLock::new()),
            turn_id: Arc::new(Mutex::new(TurnState::default())),
            asks: Arc::new(Mutex::new(HashMap::new())),
            picks: Arc::new(Mutex::new(Picks::default())),
            events: Arc::new(Mutex::new(Some(tx.clone()))),
        };
        let model = Arc::new(OnceLock::new());
        let mapper = Mapper::new(
            Arc::clone(&wire.thread_id),
            Arc::clone(&model),
            Arc::clone(&wire.turn_id),
        );
        let mut mapper = mapper;
        mapper.replayed = !matches!(open, ThreadOpen::Start);
        let reader_wire = wire.clone();
        let reader = std::thread::spawn(move || read_loop(stdout, reader_wire, tx, mapper));

        let mut bridge = Self {
            child,
            events: rx,
            reader: Some(reader),
            wire,
            model,
        };

        bridge.handshake(cwd, open)?;

        Ok(bridge)
    }

    /// `initialize` → `initialized` → `thread/start` (or `thread/resume` / `thread/fork`),
    /// capturing `thread.id` and emitting [`AgentEvent::SessionStarted`] once it's known, then —
    /// best-effort — `model/list` for the model and effort pickers.
    fn handshake(&mut self, cwd: &Path, open: ThreadOpen) -> crate::Result<()> {
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

        let cwd = cwd.display().to_string();
        let (method, params) = match &open {
            ThreadOpen::Start => ("thread/start", json!({"cwd": cwd})),
            // `excludeTurns`: the transcript is Ubiq's own record; the history is not replayed.
            ThreadOpen::Resume(id) => (
                "thread/resume",
                json!({"threadId": id, "cwd": cwd, "excludeTurns": true}),
            ),
            ThreadOpen::Fork(id) => (
                "thread/fork",
                json!({"threadId": id, "cwd": cwd, "excludeTurns": true}),
            ),
        };
        let thread_resp = self.request(method, params)?;
        let thread_id = thread_resp
            .get("thread")
            .and_then(|t| t.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("{method} response missing thread.id"))?
            .to_string();
        // Set once, here; every later reader (`send`, a `CodexInputSink`)
        // only ever sees it fully formed.
        let _ = self.wire.thread_id.set(thread_id.clone());

        // `ThreadStartResponse` states the model and the sandbox the thread runs under
        // (`codex app-server generate-ts`, 0.161.0). The sandbox is the closest thing Codex has
        // to the mode a profile picks — `Codex::modes` are its kebab-case names — so it is
        // reported under that name; a policy with no mode of that name reports none.
        let model = thread_resp
            .get("model")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .map(str::to_string);
        if let Some(model) = &model {
            let _ = self.model.set(model.clone());
        }
        let mode = thread_resp
            .pointer("/sandbox/type")
            .and_then(Value::as_str)
            .and_then(sandbox_mode);
        let effort = thread_resp
            .get("reasoningEffort")
            .and_then(Value::as_str)
            .map(str::to_string);
        // Codex names no tool or subagent list up front, so those stay empty rather than guessed.
        self.wire.emit(AgentEvent::SessionStarted {
            session_id: Some(thread_id),
            model: model.clone(),
            mode,
            tools: Vec::new(),
            agents: Vec::new(),
        });

        // The pickers: best-effort, because a conversation without them is still a conversation.
        match self.request("model/list", json!({"limit": 100})) {
            Ok(list) => {
                let options = {
                    let Ok(mut picks) = self.wire.picks.lock() else {
                        return Ok(());
                    };
                    picks.models = list
                        .get("data")
                        .and_then(Value::as_array)
                        .map(|models| {
                            models
                                .iter()
                                .filter(|m| m.get("hidden").and_then(Value::as_bool) != Some(true))
                                .cloned()
                                .collect()
                        })
                        .unwrap_or_default();
                    picks.model = model;
                    picks.effort = effort;
                    config_options(&picks)
                };
                if !options.is_empty() {
                    self.wire.emit(AgentEvent::ConfigOptionUpdate { options });
                }
            }
            Err(error) => tracing::debug!(%error, "codex model/list unanswered; no pickers"),
        }

        Ok(())
    }

    /// Send a JSON-RPC *request* (`method` + `params`, with a fresh `id`)
    /// and block (with [`REQUEST_TIMEOUT`]) for its matching response.
    fn request(&self, method: &str, params: Value) -> crate::Result<Value> {
        self.wire.request(method, params)
    }

    /// Send a JSON-RPC *notification* (`method` + `params`, no `id`) —
    /// fire-and-forget, no response expected.
    fn notify(&self, method: &str, params: Value) -> crate::Result<()> {
        self.wire.notify(method, params)
    }
}

/// The model and reasoning-effort pickers, from the catalogue and what is current: one `model`
/// select over every listed model, one `reasoning_effort` select over what the current model
/// supports. Empty where `model/list` gave nothing.
fn config_options(picks: &Picks) -> Vec<super::ConfigOption> {
    use super::{ConfigCategory, ConfigChoice, ConfigOption, ConfigValue};
    let slug = |m: &Value| {
        m.get("model")
            .or_else(|| m.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let current = picks.model.clone().or_else(|| {
        picks
            .models
            .iter()
            .find(|m| m.get("isDefault").and_then(Value::as_bool) == Some(true))
            .and_then(slug)
    });
    let Some(current) = current else {
        return Vec::new();
    };
    let mut options = Vec::new();
    let choices: Vec<ConfigChoice> = picks
        .models
        .iter()
        .filter_map(|m| {
            let value = slug(m)?;
            Some(ConfigChoice {
                name: m
                    .get("displayName")
                    .and_then(Value::as_str)
                    .unwrap_or(&value)
                    .to_string(),
                description: m
                    .get("description")
                    .and_then(Value::as_str)
                    .filter(|d| !d.is_empty())
                    .map(str::to_string),
                value,
                group: None,
            })
        })
        .collect();
    if !choices.is_empty() {
        options.push(ConfigOption {
            id: "model".to_string(),
            name: "Model".to_string(),
            description: None,
            category: Some(ConfigCategory::Model),
            value: ConfigValue::Select {
                current_value: current.clone(),
                options: choices,
            },
        });
    }
    let entry = picks
        .models
        .iter()
        .find(|m| slug(m).as_deref() == Some(current.as_str()));
    let efforts: Vec<ConfigChoice> = entry
        .and_then(|m| m.get("supportedReasoningEfforts"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let value = e
                .get("reasoningEffort")
                .and_then(Value::as_str)?
                .to_string();
            Some(ConfigChoice {
                name: value.clone(),
                description: e
                    .get("description")
                    .and_then(Value::as_str)
                    .filter(|d| !d.is_empty())
                    .map(str::to_string),
                value,
                group: None,
            })
        })
        .collect();
    let effort = picks
        .effort
        .clone()
        .filter(|e| efforts.iter().any(|c| &c.value == e))
        .or_else(|| {
            entry
                .and_then(|m| m.get("defaultReasoningEffort"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    if let (false, Some(effort)) = (efforts.is_empty(), effort) {
        options.push(ConfigOption {
            id: "reasoning_effort".to_string(),
            name: "Reasoning".to_string(),
            description: None,
            category: Some(ConfigCategory::ThoughtLevel),
            value: ConfigValue::Select {
                current_value: effort,
                options: efforts,
            },
        });
    }
    options
}

impl IoBridge for CodexBridge {
    fn send(&mut self, input: AgentInput) -> crate::Result<()> {
        write_input(&self.wire, input)
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
            wire: self.wire.clone(),
        }))
    }

    /// Kill-by-pid over the child this bridge owns; see [`crate::io::ProcessKill`].
    fn killer(&self) -> Option<Arc<dyn crate::io::AgentKill>> {
        Some(Arc::new(crate::io::ProcessKill::new(&self.child)))
    }
}

impl Drop for CodexBridge {
    fn drop(&mut self) {
        // Close stdin first (best-effort "please stop" signal), then give
        // the child a bounded window to drain/exit before killing it.
        if let Ok(mut guard) = self.wire.stdin.lock() {
            *guard = None;
        }
        // The bridge's own way onto the event stream goes too, so only the reader holds one.
        if let Ok(mut guard) = self.wire.events.lock() {
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
fn write_input(wire: &Wire, input: AgentInput) -> crate::Result<()> {
    let thread_id = wire.thread();
    match input {
        AgentInput::Prompt { .. } => {
            let text = input.prompt_text().unwrap_or_default();
            let user_input = json!([{"type": "text", "text": text, "text_elements": []}]);

            // The transcript's user block is synthesized here, as `io/jsonl` does for Claude:
            // the App Server's `userMessage` item is dropped by the reader, so this is the only
            // source. Emitted before the request so it sorts ahead of the turn's own events.
            if !text.is_empty() {
                wire.emit(AgentEvent::UserMessageChunk {
                    content: Content::text(text.clone()),
                    message_id: None,
                });
            }

            // A prompt into a running turn steers it: `turn/steer` appends the input to the turn
            // in flight, which must be the one named (`expectedTurnId`). A turn that ended in
            // between refuses, and the prompt then opens a turn of its own.
            let live = wire.turn_id.lock().ok().and_then(|slot| slot.id.clone());
            if let Some(live) = live {
                match wire.request(
                    "turn/steer",
                    json!({"threadId": thread_id, "input": user_input, "expectedTurnId": live}),
                ) {
                    Ok(_) => {
                        merge_into_live(wire, &live);
                        return Ok(());
                    }
                    Err(error) => {
                        tracing::debug!(%error, "codex refused turn/steer; starting a turn")
                    }
                }
            }

            // A model or effort picked since the last turn rides on this one; `turn/start`'s
            // overrides hold "for this turn and subsequent turns", so repeating them is harmless.
            let mut params = json!({"threadId": thread_id, "input": user_input});
            if let Ok(picks) = wire.picks.lock() {
                if let Some(model) = &picks.model {
                    params["model"] = json!(model);
                }
                if let Some(effort) = &picks.effort {
                    params["effort"] = json!(effort);
                }
            }
            // Block only on `turn/start`'s ack (which carries `turn.id`), NOT on turn completion
            // — that arrives later as `turn/completed`, read back via `next_event`.
            let started = wire.request("turn/start", params)?;
            // The ack's `turn.id` is the only place the live turn is named, and `turn/interrupt`
            // and `turn/steer` need it — so remember it here. The reader clears it when the turn
            // completes.
            if let Some(id) = started
                .get("turn")
                .and_then(|t| t.get("id"))
                .and_then(Value::as_str)
            {
                let (same, finished) = wire
                    .turn_id
                    .lock()
                    .map(|slot| {
                        (
                            slot.id.as_deref() == Some(id),
                            slot.done.as_deref() == Some(id),
                        )
                    })
                    .unwrap_or_default();
                if same {
                    // The server steered the live turn instead of opening one.
                    merge_into_live(wire, id);
                } else if !finished && let Ok(mut slot) = wire.turn_id.lock() {
                    // (A turn already seen completing is not live: its end beat this ack.)
                    slot.id = Some(id.to_string());
                }
            }
            Ok(())
        }
        AgentInput::AnswerPermission {
            request_id,
            outcome,
            ..
        } => {
            let ask = wire
                .asks
                .lock()
                .ok()
                .and_then(|mut asks| asks.remove(&request_id));
            let Some(ask) = ask else {
                // Already answered, or withdrawn by the app-server when its turn ended.
                tracing::debug!(request_id, "codex answer for no waiting request");
                return Ok(());
            };
            let choice = match &outcome {
                super::PermissionOutcome::Selected { option_id } => Some(option_id.as_str()),
                super::PermissionOutcome::Cancelled => None,
            };
            answer_ask(wire, &ask, choice)
        }
        AgentInput::Cancel => {
            // Every request still waiting is answered `cancel` first — the vocabulary's rule for a
            // cancelled turn — then the turn is interrupted.
            let waiting: Vec<Ask> = wire
                .asks
                .lock()
                .map(|mut asks| asks.drain().map(|(_, ask)| ask).collect())
                .unwrap_or_default();
            for ask in &waiting {
                let _ = answer_ask(wire, ask, None);
            }
            // `turn/interrupt` aborts the running turn and leaves the thread — and the process —
            // alive for the next `turn/start` (`_docs/harness/codex.md` §"Process lifecycle").
            // It needs both ids, and the turn id is only ever stated on `turn/start`'s ack, so a
            // cancel with no turn on record has nothing to interrupt and says so rather than
            // guessing.
            let live = wire.turn_id.lock().ok().and_then(|mut slot| slot.id.take());
            let Some(live) = live else {
                tracing::debug!("codex cancel with no turn on record; nothing to interrupt");
                return Ok(());
            };
            // Codex answers `no active turn to interrupt` when the turn ended between the
            // caller's decision and this write — a race, not a fault, so it is logged rather
            // than returned: the caller asked for the turn to be over and it is.
            // Fire-and-forget: this runs on the coordinator's thread, and the turn's end arrives as
            // `turn/completed` whatever the reply says. The reply is read on a thread of its own.
            let wire = wire.clone();
            let params = json!({"threadId": thread_id, "turnId": live});
            std::thread::spawn(move || {
                if let Err(error) = wire.request("turn/interrupt", params) {
                    tracing::debug!(%error, "codex refused turn/interrupt");
                }
            });
            Ok(())
        }
        AgentInput::Shutdown => {
            // Teardown, matching `_docs/harness/codex.md` §"Process lifecycle": "close stdin to
            // signal the app-server to stop" — the same mechanism [`Drop`] uses. The reader
            // thread keeps draining stdout until the process actually exits.
            if let Ok(mut guard) = wire.stdin.lock() {
                *guard = None;
            }
            Ok(())
        }
        AgentInput::SetConfigOption { config_id, value } => {
            let super::ConfigSetting::Text(value) = value else {
                anyhow::bail!("codex's '{config_id}' takes a choice, not a switch");
            };
            let options = {
                let mut picks = wire
                    .picks
                    .lock()
                    .map_err(|_| anyhow::anyhow!("codex picks lock poisoned"))?;
                match config_id.as_str() {
                    // A new model keeps the effort only where the new model supports it;
                    // `config_options` falls back to the model's own default otherwise.
                    "model" => picks.model = Some(value),
                    "reasoning_effort" => picks.effort = Some(value),
                    other => anyhow::bail!("codex has no '{other}' to change"),
                }
                config_options(&picks)
            };
            // Re-sent whole, so a picker drawn from the old model's efforts is replaced.
            if let Some(effort) = options.iter().find(|o| o.id == "reasoning_effort")
                && let super::ConfigValue::Select { current_value, .. } = &effort.value
                && let Ok(mut picks) = wire.picks.lock()
            {
                picks.effort = Some(current_value.clone());
            }
            wire.emit(AgentEvent::ConfigOptionUpdate { options });
            Ok(())
        }
    }
}

/// Record that a prompt joined the live turn `live`. If that turn's end was already read, the end
/// the host is owed for the prompt is emitted now instead of at `turn/completed`.
fn merge_into_live(wire: &Wire, live: &str) {
    let ended = match wire.turn_id.lock() {
        Ok(mut slot) if slot.id.as_deref() == Some(live) => {
            slot.merged += 1;
            false
        }
        _ => true,
    };
    if ended {
        wire.emit(AgentEvent::TurnEnded {
            stop_reason: StopReason::EndTurn,
            error: None,
        });
    }
}

/// The reader thread body: scan `stdout` line-by-line (newline-delimited
/// JSON-RPC), and route each parsed line by shape:
/// - a **response** to one of our requests (`id` + (`result` or `error`),
///   no `method`) → deliver to the waiting [`rpc_request`] call via
///   `pending`;
/// - a **server→client request** (`id` **and** `method`) → an approval or a question is parked
///   in `asks` and surfaced as an [`AgentEvent::PermissionRequest`], answered later by
///   [`AgentInput::AnswerPermission`]; anything else is refused at once, so Codex never waits on
///   a request nobody will answer;
/// - a **notification** (`method`, no `id`) → [`Mapper::map`] to zero
///   or more [`AgentEvent`]s.
///
/// Returns (dropping `tx`, which closes the event channel) on stdout EOF, a
/// channel disconnect (nobody left to receive), or a poisoned lock.
fn read_loop(stdout: ChildStdout, wire: Wire, tx: mpsc::Sender<AgentEvent>, mapper: Mapper) {
    read_stream(stdout, &wire, &tx, mapper);
    // The writers' way onto the stream goes with the reader, so the channel closes once the
    // child has gone.
    if let Ok(mut guard) = wire.events.lock() {
        *guard = None;
    }
}

fn read_stream(
    stdout: ChildStdout,
    wire: &Wire,
    tx: &mpsc::Sender<AgentEvent>,
    mut mapper: Mapper,
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
                let sender = match wire.pending.lock() {
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
                let ask = Ask {
                    id: id_val,
                    method: method_name.to_string(),
                    params: value.get("params").cloned().unwrap_or(Value::Null),
                };
                let (events, answer_now) = park_ask(wire, ask);
                for ev in events {
                    tracing::debug!(event = ?ev, "codex event");
                    if tx.send(ev).is_err() {
                        return;
                    }
                }
                if let Some(response) = answer_now
                    && write_line(&wire.stdin, &response).is_err()
                {
                    return;
                }
            }
            // A notification: method, no id.
            (None, Some(method_name)) => {
                // A request the app-server withdrew (its turn ended) is no longer answerable.
                if method_name == "serverRequest/resolved"
                    && let Some(id) = value.pointer("/params/requestId")
                    && let Ok(mut asks) = wire.asks.lock()
                {
                    asks.remove(&request_key(id));
                }
                for ev in mapper.map(&value) {
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

/// A JSON-RPC id as the string a [`AgentEvent::PermissionRequest`] carries.
fn request_key(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A server→client request, read off the wire: an approval or a question is parked in
/// `wire.asks` and surfaced as a [`AgentEvent::PermissionRequest`], drawn on the tool call it
/// authorises (`params.itemId`), and answered when [`AgentInput::AnswerPermission`] names it.
/// Whether Codex asks at all is its `approval_policy` — `never` under the unattended mode, so a
/// `danger-full-access` run never reaches here (`D209`).
///
/// Returns the events to emit and, for a request nobody can answer, the response to write at
/// once: a question Ubiq cannot draw gets an empty answer, and a request of a kind this bridge
/// does not serve (`item/tool/call`, `account/chatgptAuthTokens/refresh`, `attestation/generate`)
/// gets a method-not-found error — so Codex never waits on silence.
fn park_ask(wire: &Wire, ask: Ask) -> (Vec<AgentEvent>, Option<Value>) {
    let key = request_key(&ask.id);
    let Some((tool_call, options)) = ask_prompt(&ask) else {
        let response = match ask.method.as_str() {
            "item/tool/requestUserInput" => {
                json!({"jsonrpc": "2.0", "id": ask.id, "result": {"answers": {}}})
            }
            other => json!({"jsonrpc": "2.0", "id": ask.id, "error": {
                "code": -32601,
                "message": format!("agent-manager does not serve '{other}'"),
            }}),
        };
        let note = AgentEvent::Log {
            level: "warn".to_string(),
            message: format!(
                "codex asked '{}', which this bridge cannot put to a person",
                ask.method
            ),
        };
        return (vec![note], Some(response));
    };
    if let Ok(mut asks) = wire.asks.lock() {
        asks.insert(key.clone(), ask);
    }
    (
        vec![AgentEvent::PermissionRequest {
            request_id: key,
            tool_call,
            options,
        }],
        None,
    )
}

/// The prompt a parked request draws: the tool call it is about, and the buttons. `None` for a
/// request no button can answer.
fn ask_prompt(ask: &Ask) -> Option<(ToolCallUpdate, Vec<PermissionOption>)> {
    let p = &ask.params;
    let text = |key: &str| p.get(key).and_then(Value::as_str).filter(|s| !s.is_empty());
    let option = |id: &str, name: &str, kind: PermissionKind| PermissionOption {
        option_id: id.to_string(),
        name: name.to_string(),
        kind,
    };
    let allow_deny = |session: &str| {
        vec![
            option("accept", "Allow", PermissionKind::AllowOnce),
            option("acceptForSession", session, PermissionKind::AllowAlways),
            option("decline", "Deny", PermissionKind::RejectOnce),
        ]
    };
    let (kind, title, options) = match ask.method.as_str() {
        "item/commandExecution/requestApproval" | "execCommandApproval" => {
            let command = text("command")
                .map(str::to_string)
                .or_else(|| {
                    p.get("command").and_then(Value::as_array).map(|argv| {
                        argv.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                })
                .unwrap_or_else(|| "Run a command".to_string());
            (
                ToolKind::Execute,
                command,
                allow_deny("Allow for this session"),
            )
        }
        "item/fileChange/requestApproval" | "applyPatchApproval" => (
            ToolKind::Edit,
            match text("grantRoot") {
                Some(root) => format!("Allow writes under {root}"),
                None => "Apply a file change".to_string(),
            },
            allow_deny("Allow for this session"),
        ),
        "item/permissions/requestApproval" => (
            ToolKind::Other,
            match text("reason") {
                Some(reason) => format!("Grant permissions: {reason}"),
                None => "Grant permissions".to_string(),
            },
            vec![
                option("accept", "Allow for this turn", PermissionKind::AllowOnce),
                option(
                    "acceptForSession",
                    "Allow for this session",
                    PermissionKind::AllowAlways,
                ),
                option("decline", "Deny", PermissionKind::RejectOnce),
            ],
        ),
        // Accepting sends no form content, so only a request that needs none is offered Allow: a
        // `url` one (the user opens a link), or a form with no required property. Any other is
        // refused at once rather than accepted empty.
        "mcpServer/elicitation/request" if !elicitation_needs_no_content(p) => return None,
        "mcpServer/elicitation/request" => (
            ToolKind::Other,
            text("message")
                .unwrap_or("An MCP server asks to continue")
                .to_string(),
            vec![
                option("accept", "Allow", PermissionKind::AllowOnce),
                option("decline", "Deny", PermissionKind::RejectOnce),
            ],
        ),
        // One question with fixed answers is a choice of buttons; anything else (free text,
        // several questions, a secret) is not something a permission prompt can carry.
        "item/tool/requestUserInput" => {
            let questions = p.get("questions").and_then(Value::as_array)?;
            let [question] = questions.as_slice() else {
                return None;
            };
            if question.get("isSecret").and_then(Value::as_bool) == Some(true) {
                return None;
            }
            let answers: Vec<PermissionOption> = question
                .get("options")
                .and_then(Value::as_array)?
                .iter()
                .filter_map(|o| o.get("label").and_then(Value::as_str))
                // Not `AllowOnce`: an answer is not a permission, and an unattended path (the
                // host's accept-all, the CLI) takes the allowing option, so a first-listed answer
                // would be picked for the user. `RejectOnce` is the one kind they leave alone.
                .map(|label| option(label, label, PermissionKind::RejectOnce))
                .collect();
            if answers.is_empty() {
                return None;
            }
            let title = question
                .get("question")
                .or_else(|| question.get("header"))
                .and_then(Value::as_str)
                .unwrap_or("Codex asks")
                .to_string();
            (ToolKind::Other, title, answers)
        }
        _ => return None,
    };
    let tool_call = ToolCallUpdate {
        // Joined to the call it authorises; a request about no item stands on its own id.
        id: text("itemId")
            .or_else(|| text("callId"))
            .map(str::to_string)
            .unwrap_or_else(|| request_key(&ask.id)),
        title: Some(title),
        kind: Some(kind),
        status: Some(ToolStatus::Pending),
        raw_input: Some(p.clone()),
        ..ToolCallUpdate::default()
    };
    Some((tool_call, options))
}

/// Whether an `mcpServer/elicitation/request` can be accepted with no content: a `url` elicitation,
/// or a form whose schema requires nothing.
fn elicitation_needs_no_content(params: &Value) -> bool {
    match params.get("mode").and_then(Value::as_str) {
        Some("url") => true,
        Some("form") => params
            .pointer("/requestedSchema/required")
            .and_then(Value::as_array)
            .is_none_or(|required| required.is_empty()),
        _ => false,
    }
}

/// Write the answer to a parked request: `choice` is the picked option id, `None` a cancel.
fn answer_ask(wire: &Wire, ask: &Ask, choice: Option<&str>) -> crate::Result<()> {
    write_line(
        &wire.stdin,
        &json!({"jsonrpc": "2.0", "id": ask.id, "result": ask_result(ask, choice)}),
    )
}

/// The `result` each request kind expects, per its generated `*Response` type (0.161.0).
fn ask_result(ask: &Ask, choice: Option<&str>) -> Value {
    match ask.method.as_str() {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            let decision = match choice {
                Some("accept") => "accept",
                Some("acceptForSession") => "acceptForSession",
                Some("decline") => "decline",
                _ => "cancel",
            };
            json!({"decision": decision})
        }
        "execCommandApproval" | "applyPatchApproval" => {
            let decision = match choice {
                Some("accept") => json!("approved"),
                Some("acceptForSession") => json!("approved_for_session"),
                Some("decline") => json!({"denied": {"rejection": "declined by the user"}}),
                _ => json!("abort"),
            };
            json!({"decision": decision})
        }
        "item/permissions/requestApproval" => {
            // A grant is what was asked for, at the picked scope; a refusal grants nothing.
            let granted = match choice {
                Some("accept" | "acceptForSession") => {
                    let mut granted = serde_json::Map::new();
                    for key in ["network", "fileSystem"] {
                        if let Some(value) = ask.params.pointer(&format!("/permissions/{key}"))
                            && !value.is_null()
                        {
                            granted.insert(key.to_string(), value.clone());
                        }
                    }
                    Value::Object(granted)
                }
                _ => json!({}),
            };
            let scope = if choice == Some("acceptForSession") {
                "session"
            } else {
                "turn"
            };
            json!({"permissions": granted, "scope": scope})
        }
        "mcpServer/elicitation/request" => {
            let action = match choice {
                Some("accept") => "accept",
                Some("decline") => "decline",
                _ => "cancel",
            };
            let content = match ask.params.get("mode").and_then(Value::as_str) {
                Some("form") if action == "accept" => json!({}),
                _ => Value::Null,
            };
            json!({"action": action, "content": content, "_meta": null})
        }
        "item/tool/requestUserInput" => {
            let question = ask
                .params
                .pointer("/questions/0/id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match choice {
                Some(label) => json!({"answers": {question: {"answers": [label]}}}),
                None => json!({"answers": {}}),
            }
        }
        _ => Value::Null,
    }
}

/// The reader thread's notification state: which thread is the conversation's own, which items
/// already streamed, and the last cumulative usage — everything a mapping needs that one frame
/// does not carry.
///
/// Vocabulary is the app-server's v2 protocol as codex-cli 0.161.0 generates it
/// (`codex app-server generate-json-schema`); the legacy `codex/event` dialect is still read.
/// Unknown methods and item types map to nothing.
pub(crate) struct Mapper {
    /// The conversation's own thread, set once the handshake's `thread/start` returns.
    root: Arc<OnceLock<String>>,
    /// The model `thread/start` named; the mapper's own copy follows `model/rerouted`.
    started_model: Arc<OnceLock<String>>,
    rerouted_model: Option<String>,
    /// Items whose text arrived as deltas, so their `item/completed` does not say it twice.
    /// Keyed by owner (`None` the conversation, `Some` a subagent's thread) and item id.
    streamed: HashSet<(Option<String>, String)>,
    /// Each thread's last cumulative `tokenUsage.total`, so each report bills its delta. Keyed by
    /// thread id; the root is keyed by its id like any other.
    last_total: HashMap<String, Spend>,
    /// The conversation's own occupancy and window, as its last report stated them — what a
    /// subagent's report repeats, because a delegate's spend never moves the parent's ring.
    ring: (u64, u64),
    /// The thread was resumed or forked: the root's first usage report is its history.
    replayed: bool,
    /// Every subagent thread seen, by thread id.
    children: HashMap<String, Delegate>,
    /// The last `codex` rate-limit reading, merged from sparse pushes.
    limits: Limits,
    /// The live turn's id, cleared when the conversation's own turn completes — so a prompt
    /// after it opens a turn rather than steering one that is over.
    turn: TurnSlot,
}

/// One spawned agent: a thread of its own, drawn as a `Delegate` call whose id *is* the thread id —
/// the one name every one of its frames carries, so no frame waits on the spawning call to be
/// joined. `origin` is what each of its lines is stamped with.
struct Delegate {
    origin: Origin,
    /// Its spawning call has been announced (a frame can name the thread before anything else).
    announced: bool,
}

/// The `codex` limit's two windows and its block state, as the last push left them.
#[derive(Default)]
struct Limits {
    five_hour: Option<super::RateLimitWindow>,
    seven_day: Option<super::RateLimitWindow>,
    reached: Option<String>,
}

impl Mapper {
    fn new(root: Arc<OnceLock<String>>, model: Arc<OnceLock<String>>, turn: TurnSlot) -> Self {
        Self {
            root,
            started_model: model,
            rerouted_model: None,
            streamed: HashSet::new(),
            last_total: HashMap::new(),
            ring: (0, 0),
            replayed: false,
            children: HashMap::new(),
            limits: Limits::default(),
            turn,
        }
    }

    /// Map one parsed JSON-RPC *notification* (`method` + `params`, no `id`) to zero or more
    /// [`AgentEvent`]s.
    pub(crate) fn map(&mut self, value: &Value) -> Vec<AgentEvent> {
        let Some(method) = value.get("method").and_then(Value::as_str) else {
            return Vec::new();
        };
        let params = value.get("params").cloned().unwrap_or(Value::Null);

        if method == "codex/event" {
            return map_legacy_event(&params);
        }
        if method == "account/rateLimits/updated" {
            return self.map_rate_limits(params.get("rateLimits"));
        }
        // A spawned subagent runs on a thread of its own, and the app-server attaches every new
        // thread to every initialized connection (`codex-rs/app-server/src/lib.rs`, the
        // `thread_created` listener) — so its items, usage and `turn/completed` arrive here too,
        // and are the delegate's, never the conversation's.
        if let Some(child) = self.child_thread(&params) {
            return self.map_child(&child, method, &params);
        }

        let root = Origin::default();
        match method {
            "item/started" => self.map_item(&params, true, &root),
            "item/completed" => self.map_item(&params, false, &root),
            "item/agentMessage/delta" => self.delta(&params, false, &root),
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                self.delta(&params, true, &root)
            }
            "turn/plan/updated" => map_plan(&params),
            "thread/tokenUsage/updated" => self.map_usage(&params, None),
            "model/rerouted" => self.map_rerouted(&params),
            "turn/completed" => {
                // Only the conversation's own items: a subagent's may still be streaming.
                self.streamed.retain(|(owner, _)| owner.is_some());
                let mut events = map_turn_completed(&params);
                let done = params.pointer("/turn/id").and_then(Value::as_str);
                if let Ok(mut slot) = self.turn.lock() {
                    // A completion of an older turn leaves a newer live one alone.
                    let current = match (slot.id.as_deref(), done) {
                        (Some(live), Some(done)) => live == done,
                        _ => true,
                    };
                    if let Some(done) = done {
                        slot.done = Some(done.to_string());
                    }
                    if current {
                        slot.id = None;
                        let end = events.first().cloned();
                        for _ in 0..std::mem::take(&mut slot.merged) {
                            events.extend(end.clone());
                        }
                    }
                }
                events
            }
            "error" => map_error(&params),
            "warning" => match params.get("message").and_then(Value::as_str) {
                Some(message) => vec![AgentEvent::Log {
                    level: "warn".to_string(),
                    message: message.to_string(),
                }],
                None => Vec::new(),
            },
            // `turn/started` and `thread/status/changed` add nothing a consumer draws:
            // `turn/completed` is the one authoritative end of a turn, and mapping the idle
            // status as well ended every turn twice.
            _ => Vec::new(),
        }
    }

    /// The subagent thread a v2 notification belongs to, or `None` for the conversation's own. A
    /// frame naming no thread, or one read before the handshake named the root, is the root's.
    fn child_thread(&self, params: &Value) -> Option<String> {
        let thread = params
            .get("threadId")
            .and_then(Value::as_str)
            .or_else(|| params.pointer("/thread/id").and_then(Value::as_str))?;
        let root = self.root.get()?;
        (thread != root).then(|| thread.to_string())
    }

    /// Make sure a subagent thread has its `Delegate` call, learning what `thread` (a `Thread`
    /// object, from `thread/started`) says about it. Announced once, under the stamp of the
    /// thread that spawned it, so a grandchild sits in its parent delegate's transcript.
    fn announce(&mut self, thread: &str, info: Option<&Value>) -> Vec<AgentEvent> {
        let field = |key: &str| {
            info.and_then(|t| t.get(key))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let child = self
            .children
            .entry(thread.to_string())
            .or_insert_with(|| Delegate {
                origin: Origin {
                    parent_tool_use_id: Some(thread.to_string()),
                    ..Origin::default()
                },
                announced: false,
            });
        if child.origin.subagent_type.is_none() {
            child.origin.subagent_type = field("agentRole").or_else(|| field("agentNickname"));
        }
        if child.origin.model.is_none() {
            child.origin.model = field("model");
        }
        if child.origin.thinking.is_none() {
            child.origin.thinking = field("reasoningEffort");
        }
        if child.announced {
            return Vec::new();
        }
        child.announced = true;
        let title = field("agentNickname")
            .or_else(|| field("agentRole"))
            .unwrap_or_else(|| "Subagent".to_string());
        let parent = field("parentThreadId");
        let mut call = ToolCall::new(thread, title);
        call.kind = ToolKind::Delegate;
        call.status = ToolStatus::InProgress;
        call.origin = parent
            .and_then(|parent| self.children.get(&parent))
            .map(|parent| parent.origin.clone())
            .unwrap_or_default();
        vec![AgentEvent::ToolCall { call }]
    }

    /// A notification on a subagent's thread: its lines under its stamp, its turns as the
    /// status of its `Delegate` call, its usage as delegate spend. Never the conversation's end.
    fn map_child(&mut self, child: &str, method: &str, params: &Value) -> Vec<AgentEvent> {
        let mut events = self.announce(child, params.get("thread"));
        let origin = self
            .children
            .get(child)
            .map(|c| c.origin.clone())
            .unwrap_or_default();
        match method {
            "turn/started" => events.push(AgentEvent::ToolCallUpdate {
                update: ToolCallUpdate::finished(child, ToolStatus::InProgress),
            }),
            "turn/completed" => {
                let status = match params.pointer("/turn/status").and_then(Value::as_str) {
                    Some("failed" | "interrupted") => ToolStatus::Failed,
                    _ => ToolStatus::Completed,
                };
                events.push(AgentEvent::ToolCallUpdate {
                    update: ToolCallUpdate::finished(child, status),
                });
            }
            "item/started" => events.extend(self.map_item(params, true, &origin)),
            "item/completed" => events.extend(self.map_item(params, false, &origin)),
            "item/agentMessage/delta" => events.extend(self.delta(params, false, &origin)),
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                events.extend(self.delta(params, true, &origin))
            }
            "thread/tokenUsage/updated" => events.extend(self.map_usage(params, Some(child))),
            _ => {}
        }
        events
    }

    /// A `collabAgentToolCall` — the parent's side of delegation. A `spawnAgent` is drawn as the
    /// spawned thread's own `Delegate` call (titled with the prompt it was given), not as a step
    /// of its own; `wait` and the rest are ordinary steps. Any `agentsStates` it reports move
    /// those delegates' status.
    fn map_collab(&mut self, item: &Value, started: bool, origin: &Origin) -> Vec<AgentEvent> {
        let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
        let tool = item.get("tool").and_then(Value::as_str).unwrap_or("agent");
        let receivers: Vec<String> = item
            .get("receiverThreadIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        let failed = item.get("status").and_then(Value::as_str) == Some("failed");
        let mut events = Vec::new();

        if tool == "spawnAgent" && !(failed && receivers.is_empty()) {
            let sender = item.get("senderThreadId").and_then(Value::as_str);
            let title = item
                .get("prompt")
                .and_then(Value::as_str)
                .map(prompt_title)
                .filter(|t| !t.is_empty());
            for receiver in &receivers {
                let info = json!({
                    "parentThreadId": sender,
                    "model": item.get("model"),
                    "reasoningEffort": item.get("reasoningEffort"),
                });
                events.extend(self.announce(receiver, Some(&info)));
                if let Some(title) = &title {
                    events.push(AgentEvent::ToolCallUpdate {
                        update: ToolCallUpdate {
                            id: receiver.clone(),
                            title: Some(title.clone()),
                            raw_input: item.get("prompt").cloned(),
                            ..ToolCallUpdate::default()
                        },
                    });
                }
            }
        } else if started {
            let mut call = ToolCall::new(id, collab_title(tool, receivers.len()));
            call.status = ToolStatus::InProgress;
            call.raw_input = Some(item.clone());
            call.origin = origin.clone();
            events.push(AgentEvent::ToolCall { call });
        } else if !(tool == "spawnAgent" && receivers.is_empty()) {
            events.push(AgentEvent::ToolCallUpdate {
                update: ToolCallUpdate {
                    raw_output: Some(item.clone()),
                    ..ToolCallUpdate::finished(id, item_status(item))
                },
            });
        }

        if let Some(states) = item.get("agentsStates").and_then(Value::as_object) {
            for (thread, state) in states {
                if !self.children.contains_key(thread) {
                    continue;
                }
                let status = match state.get("status").and_then(Value::as_str) {
                    Some("pendingInit" | "running") => ToolStatus::InProgress,
                    Some("completed") => ToolStatus::Completed,
                    Some(_) => ToolStatus::Failed,
                    None => continue,
                };
                events.push(AgentEvent::ToolCallUpdate {
                    update: ToolCallUpdate::finished(thread, status),
                });
            }
        }
        events
    }

    fn model(&self) -> Option<String> {
        self.rerouted_model
            .clone()
            .or_else(|| self.started_model.get().cloned())
    }

    /// `item/agentMessage/delta` and the two reasoning deltas: one chunk each, grouped by the
    /// item id, which is also how `item/completed` knows the text is already out.
    fn delta(&mut self, params: &Value, thought: bool, origin: &Origin) -> Vec<AgentEvent> {
        let Some(delta) = params.get("delta").and_then(Value::as_str) else {
            return Vec::new();
        };
        let item = params
            .get("itemId")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(item) = &item {
            self.streamed
                .insert((origin.parent_tool_use_id.clone(), item.clone()));
        }
        let content = Content::text(delta);
        let origin = origin.clone();
        vec![if thought {
            AgentEvent::AgentThoughtChunk {
                content,
                message_id: item,
                origin,
            }
        } else {
            AgentEvent::AgentMessageChunk {
                content,
                message_id: item,
                origin,
            }
        }]
    }

    /// A v2 `item/started` / `item/completed`, keyed by the item's `type` tag, stamped `origin`.
    fn map_item(&mut self, params: &Value, started: bool, origin: &Origin) -> Vec<AgentEvent> {
        let Some(item) = params.get("item") else {
            return Vec::new();
        };
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
        if item_type == "collabAgentToolCall" {
            return self.map_collab(item, started, origin);
        }
        // A delegate's compaction is not the conversation's memory line.
        if item_type == "contextCompaction" && origin.is_subagent() {
            return Vec::new();
        }
        let mut events = self.map_item_kind(item, item_type, started, origin);
        for event in &mut events {
            match event {
                AgentEvent::AgentMessageChunk { origin: o, .. }
                | AgentEvent::AgentThoughtChunk { origin: o, .. } => *o = origin.clone(),
                AgentEvent::ToolCall { call } => call.origin = origin.clone(),
                _ => {}
            }
        }
        events
    }

    /// One item's events, before they are stamped with whose they are.
    fn map_item_kind(
        &mut self,
        item: &Value,
        item_type: &str,
        started: bool,
        origin: &Origin,
    ) -> Vec<AgentEvent> {
        let owner = origin.parent_tool_use_id.clone();
        let id = item.get("id").and_then(Value::as_str).unwrap_or_default();

        match (item_type, started) {
            ("agentMessage", false) => {
                if self.streamed.remove(&(owner.clone(), id.to_string())) {
                    return Vec::new();
                }
                let text = item.get("text").and_then(Value::as_str).unwrap_or_default();
                if text.is_empty() {
                    return Vec::new();
                }
                vec![AgentEvent::AgentMessageChunk {
                    origin: Origin::default(),
                    content: Content::text(text),
                    message_id: Some(id.to_string()),
                }]
            }
            ("reasoning", false) => {
                if self.streamed.remove(&(owner.clone(), id.to_string())) {
                    return Vec::new();
                }
                let text = reasoning_text(item);
                if text.is_empty() {
                    return Vec::new();
                }
                vec![AgentEvent::AgentThoughtChunk {
                    content: Content::text(text),
                    message_id: Some(id.to_string()),
                    origin: Origin::default(),
                }]
            }
            ("commandExecution", true) => {
                let command = item
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or("command");
                let mut call = ToolCall::new(id, command);
                call.kind = ToolKind::Execute;
                call.status = ToolStatus::InProgress;
                call.raw_input = Some(item.clone());
                vec![AgentEvent::ToolCall { call }]
            }
            ("commandExecution", false) => {
                // A non-zero exit is a failure even where the status reads `completed`.
                let exit_failed = item
                    .get("exitCode")
                    .and_then(Value::as_i64)
                    .is_some_and(|code| code != 0);
                let status = match item_status(item) {
                    ToolStatus::Completed if exit_failed => ToolStatus::Failed,
                    status => status,
                };
                vec![AgentEvent::ToolCallUpdate {
                    update: finished(id, status, item.get("aggregatedOutput"), item),
                }]
            }
            ("fileChange", true) => {
                let locations = locations_from_changes(item.get("changes"));
                let title = match locations.as_slice() {
                    [one] => format!("Edit {}", one.path),
                    [] => "Edit".to_string(),
                    many => format!("Edit {} files", many.len()),
                };
                let mut call = ToolCall::new(id, title);
                call.kind = ToolKind::Edit;
                call.status = ToolStatus::InProgress;
                call.locations = locations;
                call.content = diff_content(item.get("changes"));
                call.raw_input = item.get("changes").cloned();
                vec![AgentEvent::ToolCall { call }]
            }
            ("fileChange", false) => vec![AgentEvent::ToolCallUpdate {
                update: ToolCallUpdate {
                    raw_output: Some(item.clone()),
                    ..ToolCallUpdate::finished(id, item_status(item))
                },
            }],
            ("mcpToolCall", true) | ("dynamicToolCall", true) => {
                let tool = item.get("tool").and_then(Value::as_str).unwrap_or("tool");
                let title = match item
                    .get("server")
                    .or_else(|| item.get("namespace"))
                    .and_then(Value::as_str)
                {
                    Some(server) => format!("{server}: {tool}"),
                    None => tool.to_string(),
                };
                let mut call = ToolCall::new(id, title);
                call.status = ToolStatus::InProgress;
                call.raw_input = item.get("arguments").cloned();
                vec![AgentEvent::ToolCall { call }]
            }
            ("mcpToolCall", false) | ("dynamicToolCall", false) => {
                let error = item.pointer("/error/message").cloned();
                vec![AgentEvent::ToolCallUpdate {
                    update: finished(id, item_status(item), error.as_ref(), item),
                }]
            }
            ("webSearch", true) => {
                let query = item
                    .get("query")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let mut call = ToolCall::new(id, format!("Web search: {query}"));
                call.kind = ToolKind::Fetch;
                call.status = ToolStatus::InProgress;
                vec![AgentEvent::ToolCall { call }]
            }
            ("webSearch", false) => vec![AgentEvent::ToolCallUpdate {
                update: ToolCallUpdate::finished(id, ToolStatus::Completed),
            }],
            ("contextCompaction", false) => vec![AgentEvent::Compacted],
            // The user's own turn is synthesized in `write_input` the moment it is sent, so the
            // server's `userMessage` item is dropped here: one source, never a duplicate.
            ("userMessage", _) => Vec::new(),
            _ => Vec::new(),
        }
    }

    /// `thread/tokenUsage/updated` onto the accounting contract on [`AgentEvent::UsageUpdate`].
    ///
    /// The payload is `{ total, last, modelContextWindow }`, each breakdown `{ totalTokens,
    /// inputTokens, cachedInputTokens, cacheWriteInputTokens, outputTokens,
    /// reasoningOutputTokens }`. How they overlap is read from Codex's own source, not the
    /// schema, and is unconfirmed on a live capture:
    ///
    /// - **Occupancy is `last.totalTokens`** against `modelContextWindow` — Codex's own
    ///   `TokenUsage::tokens_in_context_window` is `total_tokens` of the last response
    ///   (`codex-rs/protocol/src/protocol.rs`).
    /// - **Cached and cache-written tokens are inside `inputTokens`, and reasoning is inside
    ///   `outputTokens`**: `TokenUsage::non_cached_input` is `input − cached`, the Responses
    ///   parser's own test reads `input 100 = cached 40 + cache_write 60` with `total 110 = input
    ///   + output 10` of which `reasoning 5` (`codex-rs/codex-api/src/sse/responses.rs`). So
    ///   [`Spend`] splits them out rather than adding them on top.
    /// - **`total` is the thread's running sum** (`TokenUsageInfo::append_last_usage`), so a
    ///   report bills its difference from the previous one; the first report bills `last`, so a
    ///   resumed thread's history is not billed again.
    ///
    /// Codex states no money for a plan login, so `cost` is absent.
    ///
    /// A subagent's report (`child`) carries its own spend under its stamp and repeats the
    /// conversation's last ring — rule 4 of the contract: a delegate never moves occupancy.
    fn map_usage(&mut self, params: &Value, child: Option<&str>) -> Vec<AgentEvent> {
        let Some(usage) = params.get("tokenUsage") else {
            return Vec::new();
        };
        let (Some(total), Some(last)) = (
            usage.get("total").map(breakdown),
            usage.get("last").map(breakdown),
        ) else {
            return Vec::new();
        };
        let key = child
            .map(str::to_string)
            .or_else(|| self.root.get().cloned())
            .unwrap_or_default();
        let spend = match self.last_total.get(&key) {
            Some(previous) => total.saturating_sub(previous),
            // A resumed or forked root replays its history as its first report: a baseline.
            None if child.is_none() && self.replayed => Spend::default(),
            None => last,
        };
        self.last_total.insert(key, total);
        let spend = (spend.total() > 0).then_some(spend);

        if let Some(child) = child {
            let origin = self
                .children
                .get(child)
                .map(|c| c.origin.clone())
                .unwrap_or_default();
            let (used, size) = self.ring;
            return vec![AgentEvent::UsageUpdate {
                used,
                size,
                cost: None,
                model: origin.model.clone().or_else(|| self.model()),
                spend,
                origin,
            }];
        }

        let used = usage
            .pointer("/last/totalTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let size = usage
            .get("modelContextWindow")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        self.ring = (used, size);
        vec![AgentEvent::UsageUpdate {
            used,
            size,
            cost: None,
            model: self.model(),
            spend,
            origin: Origin::default(),
        }]
    }

    /// `account/rateLimits/updated` onto [`AgentEvent::RateLimitUpdate`] — the same gauge
    /// Claude's stream pushes, so the host files it per account and the rings draw it unchanged.
    ///
    /// Only the `codex` limit is read (a `limitId` of `codex`, or none): other metered limits
    /// are ignored for now. Windows are placed **by `windowDurationMins`**, not by position —
    /// 300 is the five-hour window and 10080 the week; one of any other length fills neither.
    /// The push is sparse (the schema says to merge it into the last reading), so a window it
    /// omits keeps its last value. `status` is `"allowed"` unless `rateLimitReachedType` names
    /// what blocked the account.
    fn map_rate_limits(&mut self, snapshot: Option<&Value>) -> Vec<AgentEvent> {
        let Some(snapshot) = snapshot else {
            return Vec::new();
        };
        match snapshot.get("limitId").and_then(Value::as_str) {
            None | Some("codex") => {}
            Some(_) => return Vec::new(),
        }
        for key in ["primary", "secondary"] {
            let Some(window) = snapshot.get(key).filter(|w| !w.is_null()) else {
                continue;
            };
            let Some((minutes, reading)) = rate_window(window) else {
                continue;
            };
            match minutes {
                FIVE_HOURS_MINS => self.limits.five_hour = Some(reading),
                WEEK_MINS => self.limits.seven_day = Some(reading),
                _ => {}
            }
        }
        if let Some(reached) = snapshot.get("rateLimitReachedType") {
            self.limits.reached = reached.as_str().map(str::to_string);
        }
        if self.limits.five_hour.is_none() && self.limits.seven_day.is_none() {
            return Vec::new();
        }
        vec![AgentEvent::RateLimitUpdate {
            five_hour: self.limits.five_hour.clone(),
            seven_day: self.limits.seven_day.clone(),
            status: self
                .limits
                .reached
                .clone()
                .unwrap_or_else(|| "allowed".to_string()),
            overage_status: None,
            overage_reason: None,
        }]
    }

    /// `model/rerouted`: later usage is the new model's, and the reader is told why.
    fn map_rerouted(&mut self, params: &Value) -> Vec<AgentEvent> {
        let Some(to) = params.get("toModel").and_then(Value::as_str) else {
            return Vec::new();
        };
        self.rerouted_model = Some(to.to_string());
        let from = params
            .get("fromModel")
            .and_then(Value::as_str)
            .unwrap_or("?");
        vec![AgentEvent::Log {
            level: "info".to_string(),
            message: format!("codex rerouted the turn from {from} to {to}"),
        }]
    }
}

/// The five-hour window's `windowDurationMins`.
const FIVE_HOURS_MINS: u64 = 300;
/// The weekly window's `windowDurationMins`.
const WEEK_MINS: u64 = 10_080;

/// One `RateLimitWindow { usedPercent, windowDurationMins, resetsAt }` as its length in minutes
/// and the library's reading. `None` when it states no length or no reset — neither is invented.
fn rate_window(window: &Value) -> Option<(u64, super::RateLimitWindow)> {
    let minutes = window.get("windowDurationMins").and_then(Value::as_u64)?;
    let used = window.get("usedPercent").and_then(Value::as_f64)?;
    let resets_at = window.get("resetsAt").and_then(Value::as_i64)?;
    Some((
        minutes,
        super::RateLimitWindow {
            utilization_pct: used.round().clamp(0.0, 100.0) as u8,
            resets_at,
        },
    ))
}

/// A delegate's title from the prompt it was spawned with: its first line, kept short.
fn prompt_title(prompt: &str) -> String {
    let line = prompt
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    match line.char_indices().nth(80) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_string(),
    }
}

/// A non-spawning collaboration step, in words.
fn collab_title(tool: &str, agents: usize) -> String {
    let verb = match tool {
        "wait" => "Wait for",
        "sendInput" | "sendMessage" => "Message",
        "followupTask" => "Follow up with",
        "resumeAgent" => "Resume",
        "interruptAgent" => "Interrupt",
        "closeAgent" => "Close",
        "listAgents" => return "List agents".to_string(),
        other => return other.to_string(),
    };
    match agents {
        1 => format!("{verb} agent"),
        n => format!("{verb} {n} agents"),
    }
}

/// One `TokenUsageBreakdown` as [`Spend`], with cached and cache-written input taken out of
/// `inputTokens` and reasoning out of `outputTokens` — see [`Mapper::map_usage`].
fn breakdown(value: &Value) -> Spend {
    let field = |name: &str| value.get(name).and_then(Value::as_u64).unwrap_or(0);
    let cache_read = field("cachedInputTokens");
    let cache_creation = field("cacheWriteInputTokens");
    let thinking = field("reasoningOutputTokens");
    Spend {
        input: field("inputTokens")
            .saturating_sub(cache_read)
            .saturating_sub(cache_creation),
        output: field("outputTokens").saturating_sub(thinking),
        thinking,
        cache_read,
        cache_creation,
    }
}

/// The codex sandbox policy's `type` under the mode name [`crate::harness::codex`] offers.
fn sandbox_mode(policy: &str) -> Option<String> {
    match policy {
        "readOnly" => Some("read-only"),
        "workspaceWrite" => Some("workspace-write"),
        "dangerFullAccess" => Some("danger-full-access"),
        _ => None,
    }
    .map(str::to_string)
}

/// An item's `status` (`inProgress | completed | failed | declined`) as a tool status.
fn item_status(item: &Value) -> ToolStatus {
    match item.get("status").and_then(Value::as_str) {
        Some("failed" | "declined" | "interrupted") => ToolStatus::Failed,
        Some("inProgress") => ToolStatus::InProgress,
        _ => ToolStatus::Completed,
    }
}

/// A finished tool call, carrying `text` (when it is a non-empty string) as its content and the
/// whole item as its raw output.
fn finished(id: &str, status: ToolStatus, text: Option<&Value>, item: &Value) -> ToolCallUpdate {
    let content = text
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(|t| {
            vec![ToolContent::Content {
                content: Content::text(t),
            }]
        });
    ToolCallUpdate {
        content,
        raw_output: Some(item.clone()),
        ..ToolCallUpdate::finished(id, status)
    }
}

/// A `fileChange`'s unified diffs, as text. Codex states a diff, not the before-and-after
/// [`ToolContent::Diff`] wants, and reconstructing either side from a hunk would be a guess.
fn diff_content(changes: Option<&Value>) -> Vec<ToolContent> {
    changes
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|change| change.get("diff").and_then(Value::as_str))
        .filter(|diff| !diff.is_empty())
        .map(|diff| ToolContent::Content {
            content: Content::text(diff),
        })
        .collect()
}

/// A completed `reasoning` item's text: the summary where there is one, else the raw content.
fn reasoning_text(item: &Value) -> String {
    let join = |key: &str| {
        item.get(key)
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n\n")
            })
            .unwrap_or_default()
    };
    let summary = join("summary");
    if summary.is_empty() {
        join("content")
    } else {
        summary
    }
}

/// `turn/plan/updated`: the whole list each time, as [`AgentEvent::Plan`] wants it.
fn map_plan(params: &Value) -> Vec<AgentEvent> {
    let Some(steps) = params.get("plan").and_then(Value::as_array) else {
        return Vec::new();
    };
    let entries = steps
        .iter()
        .filter_map(|step| {
            let content = step.get("step").and_then(Value::as_str)?.to_string();
            let status = match step.get("status").and_then(Value::as_str) {
                Some("inProgress") => PlanStatus::InProgress,
                Some("completed") => PlanStatus::Completed,
                _ => PlanStatus::Pending,
            };
            Some(PlanEntry {
                content,
                priority: PlanPriority::default(),
                status,
            })
        })
        .collect();
    vec![AgentEvent::Plan { entries }]
}

/// A stateless one-shot mapping, for tests that feed single frames with no thread context.
#[cfg(test)]
pub(crate) fn map_notification(value: &Value) -> Vec<AgentEvent> {
    Mapper::new(
        Arc::new(OnceLock::new()),
        Arc::new(OnceLock::new()),
        Arc::new(Mutex::new(TurnState::default())),
    )
    .map(value)
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
                origin: Origin::default(),
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

/// Map the root thread's `turn/completed` to the turn's end, from `turn.status`
/// (`completed | interrupted | failed`) and `turn.error { message, codexErrorInfo }`.
///
/// `codexErrorInfo` is a bare string (`contextWindowExceeded`, `usageLimitExceeded`, …) or a
/// one-key object (`{ httpConnectionFailed: { httpStatusCode } }`); its name rides on the error
/// text so a limit hit reads as one. A window overflow is [`StopReason::MaxTokens`].
fn map_turn_completed(params: &Value) -> Vec<AgentEvent> {
    let turn = params.get("turn").unwrap_or(&Value::Null);
    let info = turn
        .pointer("/error/codexErrorInfo")
        .and_then(|info| match info {
            Value::String(name) => Some(name.clone()),
            Value::Object(map) => map.keys().next().cloned(),
            _ => None,
        });
    let message = turn
        .pointer("/error/message")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty());
    let error = match (message, info.as_deref()) {
        (Some(message), Some(info)) if info != "other" => Some(format!("{message} ({info})")),
        (Some(message), _) => Some(message.to_string()),
        (None, Some(info)) => Some(info.to_string()),
        (None, None) => None,
    };
    let stop_reason = match turn.get("status").and_then(Value::as_str) {
        Some("interrupted") => StopReason::Cancelled,
        Some("failed") if info.as_deref() == Some("contextWindowExceeded") => StopReason::MaxTokens,
        Some("failed") => StopReason::Failed,
        _ => StopReason::EndTurn,
    };
    vec![AgentEvent::TurnEnded { stop_reason, error }]
}

/// Map a v2 `error` notification to a diagnostic, never to the turn's end.
///
/// A non-retrying error is recorded by the app-server as the turn's `last_error` and the turn
/// then completes with `status: failed` carrying the same error
/// (`codex-rs/app-server/src/bespoke_event_handling.rs`, `handle_error_notification` /
/// `TurnComplete`) — so `turn/completed` ends it, and ending it here too ended it twice. A
/// retrying one (`willRetry: true`, a stream error) is progress, reported at `warn`.
fn map_error(params: &Value) -> Vec<AgentEvent> {
    let will_retry = params
        .get("willRetry")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let Some(message) = params
        .pointer("/error/message")
        .or_else(|| params.get("message"))
        .and_then(Value::as_str)
    else {
        return Vec::new();
    };
    vec![AgentEvent::Log {
        level: if will_retry { "warn" } else { "error" }.to_string(),
        message: message.to_string(),
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
                origin: Origin::default(),
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
                "params":{"item":{"id":"i1","type":"agentMessage","text":"hi from v2"}}}"#,
        )
        .unwrap();
        assert_eq!(
            map_notification(&v),
            vec![AgentEvent::AgentMessageChunk {
                origin: Origin::default(),
                content: Content::text("hi from v2"),
                message_id: Some("i1".to_string()),
            }]
        );
    }

    #[test]
    fn map_notification_v2_item_started_and_completed_command_execution() {
        let started: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"item/started",
                "params":{"item":{"id":"c1","type":"commandExecution","command":"ls",
                  "status":"inProgress"}}}"#,
        )
        .unwrap();
        let events = map_notification(&started);
        let AgentEvent::ToolCall { call } = &events[0] else {
            panic!("expected a tool call, got {events:?}");
        };
        assert_eq!(call.id, "c1");
        assert_eq!(call.title, "ls");
        assert_eq!(call.kind, ToolKind::Execute);
        assert_eq!(call.status, ToolStatus::InProgress);

        let completed: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"item/completed",
                "params":{"item":{"id":"c1","type":"commandExecution","command":"ls",
                  "status":"completed","aggregatedOutput":"ok","exitCode":0}}}"#,
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

    /// `changes` is `[{ path, kind, diff }]`; each path is a location and each diff is content.
    #[test]
    fn map_notification_v2_item_started_file_change_locations_when_a_path_is_known() {
        let v: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"item/started",
                "params":{"item":{"id":"f1","type":"fileChange","status":"inProgress",
                  "changes":[{"path":"/tmp/a.rs","kind":{"type":"add"},"diff":"+x"}]}}}"#,
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

    /// A command that exits non-zero failed, whatever its `status` says.
    #[test]
    fn map_notification_v2_command_exit_code_decides_failure() {
        let v: Value = serde_json::from_str(
            r#"{"method":"item/completed","params":{"item":{"id":"c1","type":"commandExecution",
                "command":"false","status":"completed","aggregatedOutput":"","exitCode":1}}}"#,
        )
        .unwrap();
        let events = map_notification(&v);
        let AgentEvent::ToolCallUpdate { update } = &events[0] else {
            panic!("expected an update, got {events:?}");
        };
        assert_eq!(update.status, Some(ToolStatus::Failed));
        assert_eq!(update.content, None, "an empty output is no content");
    }

    /// `turn.status` and `turn.error.codexErrorInfo` decide the stop reason; usage on the turn
    /// itself is not read (it arrives as `thread/tokenUsage/updated`).
    #[test]
    fn map_notification_v2_turn_completed_status_is_the_stop_reason() {
        let turn = |turn: &str| {
            let v: Value = serde_json::from_str(&format!(
                r#"{{"method":"turn/completed","params":{{"threadId":"t","turn":{turn}}}}}"#
            ))
            .unwrap();
            map_notification(&v)
        };
        assert_eq!(
            turn(r#"{"id":"x","status":"completed","error":null}"#),
            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::EndTurn,
                error: None,
            }]
        );
        assert_eq!(
            turn(r#"{"id":"x","status":"interrupted","error":null}"#),
            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::Cancelled,
                error: None,
            }]
        );
        assert_eq!(
            turn(
                r#"{"id":"x","status":"failed","error":{"message":"401",
                    "codexErrorInfo":{"httpConnectionFailed":{"httpStatusCode":401}}}}"#
            ),
            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::Failed,
                error: Some("401 (httpConnectionFailed)".to_string()),
            }]
        );
        assert_eq!(
            turn(
                r#"{"id":"x","status":"failed","error":{"message":"full",
                    "codexErrorInfo":"contextWindowExceeded"}}"#
            ),
            vec![AgentEvent::TurnEnded {
                stop_reason: StopReason::MaxTokens,
                error: Some("full (contextWindowExceeded)".to_string()),
            }]
        );
    }

    /// The idle status follows every `turn/completed`; mapping both ended each turn twice.
    #[test]
    fn map_notification_v2_thread_status_changed_ends_nothing() {
        for status in ["idle", "active", "systemError"] {
            let v: Value = serde_json::from_str(&format!(
                r#"{{"method":"thread/status/changed","params":{{"status":{{"type":"{status}"}}}}}}"#
            ))
            .unwrap();
            assert_eq!(map_notification(&v), Vec::new(), "{status}");
        }
    }

    /// An `error` is followed by `turn/completed { status: failed }`, which is what ends the
    /// turn — so the error itself is only ever a diagnostic.
    #[test]
    fn map_notification_v2_error_is_a_log_not_an_end() {
        let terminal: Value = serde_json::from_str(
            r#"{"method":"error","params":{"error":{"message":"boom"},"willRetry":false}}"#,
        )
        .unwrap();
        assert_eq!(
            map_notification(&terminal),
            vec![AgentEvent::Log {
                level: "error".to_string(),
                message: "boom".to_string(),
            }]
        );
        let retrying: Value = serde_json::from_str(
            r#"{"method":"error","params":{"error":{"message":"Reconnecting... 2/5"},"willRetry":true}}"#,
        )
        .unwrap();
        assert!(matches!(
            &map_notification(&retrying)[..],
            [AgentEvent::Log { level, .. }] if level == "warn"
        ));
    }

    /// Every notification frame of a fixture, through one mapper whose root thread is `root`.
    fn replay(fixture: &str, root: &str, model: Option<&str>) -> Vec<AgentEvent> {
        let root_lock = Arc::new(OnceLock::new());
        root_lock.set(root.to_string()).unwrap();
        let model_lock = Arc::new(OnceLock::new());
        if let Some(model) = model {
            model_lock.set(model.to_string()).unwrap();
        }
        let mut mapper = Mapper::new(root_lock, model_lock, Arc::new(Mutex::new(TurnState::default())));
        fixture
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|v| v.get("method").is_some() && v.get("id").is_none())
            .flat_map(|v| mapper.map(&v))
            .collect()
    }

    /// A real 0.161.0 app-server turn with no login: ten retrying stream errors, a warning, the
    /// final error, then `turn/completed { status: failed }` — exactly one end, and it says why.
    #[test]
    fn live_unauthenticated_turn_ends_once_and_says_why() {
        let events = replay(
            include_str!(
                "../../tests/fixtures/codex-app-server/live-0.161.0-unauthenticated-turn.ndjson"
            ),
            "01a11c84-24d4-7190-bc2e-25a5e93b175d",
            None,
        );
        let ends: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::TurnEnded { .. }))
            .collect();
        assert_eq!(ends.len(), 1, "{events:?}");
        let AgentEvent::TurnEnded { stop_reason, error } = ends[0] else {
            unreachable!()
        };
        assert_eq!(*stop_reason, StopReason::Failed);
        assert!(
            error
                .as_deref()
                .is_some_and(|e| e.contains("401") && e.ends_with("(httpConnectionFailed)")),
            "{error:?}"
        );
        assert!(matches!(events.last(), Some(AgentEvent::TurnEnded { .. })));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AgentEvent::AgentMessageChunk { .. })),
            "the echoed user message is not assistant text: {events:?}"
        );
    }

    /// A schema-shaped turn: streamed reasoning and text said once, a failing command, a diff,
    /// the plan, usage against the stated window, and a subagent thread that neither speaks
    /// into the conversation nor ends its turn.
    #[test]
    fn schema_turn_maps_onto_the_accounting_contract() {
        let events = replay(
            include_str!("../../tests/fixtures/codex-app-server/schema-0.161.0-turn.ndjson"),
            "t-root",
            Some("gpt-6.1-sol"),
        );

        let ends = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::TurnEnded { .. }))
            .count();
        assert_eq!(
            ends, 1,
            "only the root's turn/completed ends it: {events:?}"
        );
        assert!(matches!(events.last(), Some(AgentEvent::TurnEnded { .. })));

        let text = |subagent: bool| -> String {
            events
                .iter()
                .filter_map(|e| match e {
                    AgentEvent::AgentMessageChunk {
                        content, origin, ..
                    } if origin.is_subagent() == subagent => content.as_text(),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(text(false), "Fixed the assertion.", "streamed once");
        assert_eq!(text(true), "child speaking", "the delegate's own line");
        let thoughts = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::AgentThoughtChunk { .. }))
            .count();
        assert_eq!(thoughts, 1);

        assert!(events.iter().any(|e| matches!(
            e,
            AgentEvent::ToolCallUpdate { update }
                if update.id == "c-1" && update.status == Some(ToolStatus::Failed)
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            AgentEvent::ToolCall { call }
                if call.id == "f-1" && call.kind == ToolKind::Edit && call.content.len() == 1
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            AgentEvent::Plan { entries } if entries.len() == 3
                && entries[1].status == PlanStatus::InProgress
        )));

        // The spawned agent is one `Delegate` call whose id is its thread, titled with the
        // prompt it was given, finished when its turn did and when `wait` reported it done.
        let AgentEvent::ToolCall { call } = events
            .iter()
            .find(|e| matches!(e, AgentEvent::ToolCall { call } if call.kind == ToolKind::Delegate))
            .expect("a delegate call")
        else {
            unreachable!()
        };
        assert_eq!((call.id.as_str(), call.title.as_str()), ("t-child", "Ada"));
        assert!(
            !call.origin.is_subagent(),
            "spawned from the conversation itself"
        );
        assert!(events.iter().any(|e| matches!(
            e,
            AgentEvent::ToolCallUpdate { update }
                if update.id == "t-child" && update.title.as_deref() == Some("Check the CI logs")
        )));
        let finished = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    AgentEvent::ToolCallUpdate { update }
                        if update.id == "t-child" && update.status == Some(ToolStatus::Completed)
                )
            })
            .count();
        assert_eq!(finished, 2, "its turn ending, then wait's agentsStates");
        assert!(events.iter().any(|e| matches!(
            e,
            AgentEvent::ToolCall { call } if call.id == "w-1" && call.title == "Wait for agent"
        )));
        assert!(
            !events.iter().any(|e| matches!(
                e,
                AgentEvent::ToolCall { call } if call.id == "s-1"
            )),
            "a spawn is the delegate's own call, not a step of its own"
        );

        // The pushed `codex` limit, windows placed by their length.
        assert!(events.contains(&AgentEvent::RateLimitUpdate {
            five_hour: Some(super::super::RateLimitWindow {
                utilization_pct: 42,
                resets_at: 1_791_490_000,
            }),
            seven_day: Some(super::super::RateLimitWindow {
                utilization_pct: 13,
                resets_at: 1_791_900_000,
            }),
            status: "allowed".to_string(),
            overage_status: None,
            overage_reason: None,
        }));

        let mut usage: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::UsageUpdate {
                    used,
                    size,
                    spend,
                    model,
                    origin,
                    cost,
                } => Some((
                    *used,
                    *size,
                    *spend,
                    model.clone(),
                    origin.clone(),
                    cost.clone(),
                )),
                _ => None,
            })
            .collect();
        assert_eq!(usage.len(), 3, "{usage:?}");
        let child = usage.remove(1);
        // The delegate's report: its own spend under its stamp, the parent's ring repeated.
        assert_eq!(
            child,
            (
                12_500,
                272_000,
                Some(Spend {
                    input: 800,
                    output: 100,
                    ..Spend::default()
                }),
                Some("gpt-6.1-luna".to_string()),
                Origin {
                    parent_tool_use_id: Some("t-child".to_string()),
                    subagent_type: Some("explorer".to_string()),
                    model: Some("gpt-6.1-luna".to_string()),
                    thinking: Some("low".to_string()),
                },
                None,
            )
        );
        let model = Some("gpt-6.1-sol".to_string());
        // First report bills `last`: 12000 in of which 8000 cached; 500 out of which 200 reasoning.
        assert_eq!(
            usage[0],
            (
                12_500,
                272_000,
                Some(Spend {
                    input: 4_000,
                    output: 300,
                    thinking: 200,
                    cache_read: 8_000,
                    cache_creation: 0,
                }),
                model.clone(),
                Origin::default(),
                None,
            )
        );
        // Second bills the change in `total`, and occupancy is the last response's size.
        assert_eq!(
            usage[1],
            (
                13_500,
                272_000,
                Some(Spend {
                    input: 1_000,
                    output: 400,
                    thinking: 100,
                    cache_read: 12_000,
                    cache_creation: 0,
                }),
                model,
                Origin::default(),
                None,
            )
        );
    }

    /// Cache writes are inside `inputTokens` too (Codex's Responses parser test reads
    /// `input 100 = cached 40 + cache_write 60`), so nothing is billed twice.
    #[test]
    fn breakdown_takes_cache_and_reasoning_out_of_their_parents() {
        let spend = breakdown(&json!({
            "totalTokens": 110, "inputTokens": 100, "cachedInputTokens": 40,
            "cacheWriteInputTokens": 60, "outputTokens": 10, "reasoningOutputTokens": 5
        }));
        assert_eq!(
            spend,
            Spend {
                input: 0,
                output: 5,
                thinking: 5,
                cache_read: 40,
                cache_creation: 60,
            }
        );
        assert_eq!(spend.total(), 110);
    }

    #[test]
    fn map_notification_unknown_method_is_ignored() {
        let v: Value =
            serde_json::from_str(r#"{"jsonrpc":"2.0","method":"something/new","params":{}}"#)
                .unwrap();
        assert_eq!(map_notification(&v), Vec::new());
    }

    fn ask(method: &str, params: Value) -> Ask {
        Ask {
            id: json!(7),
            method: method.to_string(),
            params,
        }
    }

    /// A command approval is drawn on the command's own tool call, with three buttons.
    #[test]
    fn a_command_approval_is_a_prompt_on_its_call() {
        let (call, options) = ask_prompt(&ask(
            "item/commandExecution/requestApproval",
            json!({"itemId": "c-1", "threadId": "t", "turnId": "u", "command": "rm -rf /tmp/x"}),
        ))
        .unwrap();
        assert_eq!(call.id, "c-1");
        assert_eq!(call.title.as_deref(), Some("rm -rf /tmp/x"));
        assert_eq!(call.kind, Some(ToolKind::Execute));
        assert_eq!(call.status, Some(ToolStatus::Pending));
        let kinds: Vec<_> = options
            .iter()
            .map(|o| (o.option_id.as_str(), o.kind))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("accept", PermissionKind::AllowOnce),
                ("acceptForSession", PermissionKind::AllowAlways),
                ("decline", PermissionKind::RejectOnce),
            ]
        );
    }

    /// Each kind answers in its own generated `*Response` shape; a cancel is the cancel value.
    #[test]
    fn answers_take_each_request_kinds_own_shape() {
        let cmd = ask("item/commandExecution/requestApproval", json!({}));
        assert_eq!(
            ask_result(&cmd, Some("accept")),
            json!({"decision": "accept"})
        );
        assert_eq!(
            ask_result(&cmd, Some("acceptForSession")),
            json!({"decision": "acceptForSession"})
        );
        assert_eq!(
            ask_result(&cmd, Some("decline")),
            json!({"decision": "decline"})
        );
        assert_eq!(ask_result(&cmd, None), json!({"decision": "cancel"}));

        let legacy = ask("execCommandApproval", json!({}));
        assert_eq!(
            ask_result(&legacy, Some("accept")),
            json!({"decision": "approved"})
        );
        assert_eq!(ask_result(&legacy, None), json!({"decision": "abort"}));

        let perms = ask(
            "item/permissions/requestApproval",
            json!({"permissions": {"network": {"enabled": true}, "fileSystem": null}}),
        );
        assert_eq!(
            ask_result(&perms, Some("acceptForSession")),
            json!({"permissions": {"network": {"enabled": true}}, "scope": "session"})
        );
        assert_eq!(
            ask_result(&perms, Some("decline")),
            json!({"permissions": {}, "scope": "turn"})
        );

        let elicit = ask("mcpServer/elicitation/request", json!({}));
        assert_eq!(ask_result(&elicit, None)["action"], json!("cancel"));

        let question = ask(
            "item/tool/requestUserInput",
            json!({"questions": [{"id": "q1", "question": "Which?", "isSecret": false,
                "options": [{"label": "A"}, {"label": "B"}]}]}),
        );
        let (_, options) = ask_prompt(&question).unwrap();
        assert_eq!(options.len(), 2);
        assert_eq!(
            ask_result(&question, Some("B")),
            json!({"answers": {"q1": {"answers": ["B"]}}})
        );
    }

    /// A free-text question and a request kind nobody serves are answered at once, never parked.
    #[test]
    fn unanswerable_requests_are_answered_at_once() {
        let free = ask(
            "item/tool/requestUserInput",
            json!({"questions": [{"id": "q1", "question": "Name?", "options": null}]}),
        );
        assert!(ask_prompt(&free).is_none());
        assert!(ask_prompt(&ask("item/tool/call", json!({}))).is_none());
    }

    #[test]
    fn config_options_offer_the_models_and_the_current_models_efforts() {
        let picks = Picks {
            models: vec![
                json!({"model": "a", "displayName": "A", "isDefault": true,
                    "supportedReasoningEfforts": [{"reasoningEffort": "low", "description": ""},
                        {"reasoningEffort": "high", "description": ""}],
                    "defaultReasoningEffort": "low"}),
                json!({"model": "b", "displayName": "B",
                    "supportedReasoningEfforts": [{"reasoningEffort": "medium", "description": ""}],
                    "defaultReasoningEffort": "medium"}),
            ],
            model: Some("b".to_string()),
            effort: Some("high".to_string()),
        };
        let options = config_options(&picks);
        assert_eq!(options.len(), 2);
        let super::super::ConfigValue::Select {
            current_value,
            options: efforts,
        } = &options[1].value
        else {
            panic!()
        };
        // `high` is not one of b's efforts, so b's default stands.
        assert_eq!(current_value, "medium");
        assert_eq!(efforts.len(), 1);
    }
    /// A mapper on root thread `t-1` with a shared turn slot, for the turn-bookkeeping tests.
    fn mapper_with_turn(open_replayed: bool) -> (Mapper, TurnSlot) {
        let root = Arc::new(OnceLock::new());
        root.set("t-1".to_string()).unwrap();
        let turn: TurnSlot = Arc::new(Mutex::new(TurnState::default()));
        let mut mapper = Mapper::new(root, Arc::new(OnceLock::new()), Arc::clone(&turn));
        mapper.replayed = open_replayed;
        (mapper, turn)
    }

    fn completed(turn: &str) -> Value {
        json!({"method": "turn/completed", "params": {"threadId": "t-1",
            "turn": {"id": turn, "status": "completed"}}})
    }

    /// Each prompt merged into the live turn is owed an end the host can count: Codex ends the
    /// merged turn once, so the reader adds one `TurnEnded` per merged prompt at `turn/completed`.
    #[test]
    fn a_merged_prompt_is_owed_a_turn_end() {
        let (mut mapper, turn) = mapper_with_turn(false);
        {
            let mut slot = turn.lock().unwrap();
            slot.id = Some("turn-1".to_string());
            slot.merged = 2;
        }
        let ends = mapper.map(&completed("turn-1"));
        assert_eq!(ends.len(), 3, "{ends:?}");
        assert!(ends.iter().all(|e| matches!(e, AgentEvent::TurnEnded { .. })));
        let slot = turn.lock().unwrap();
        assert_eq!((slot.id.clone(), slot.merged), (None, 0));
    }

    /// An older turn's completion leaves a newer live turn (and its merged prompts) alone, and an
    /// ack that names a turn already seen completing is not stored as live.
    #[test]
    fn a_completion_clears_only_its_own_turn() {
        let (mut mapper, turn) = mapper_with_turn(false);
        {
            let mut slot = turn.lock().unwrap();
            slot.id = Some("turn-2".to_string());
            slot.merged = 1;
        }
        let ends = mapper.map(&completed("turn-1"));
        assert_eq!(ends.len(), 1, "{ends:?}");
        {
            let slot = turn.lock().unwrap();
            assert_eq!(slot.id.as_deref(), Some("turn-2"));
            assert_eq!(slot.merged, 1);
            assert_eq!(slot.done.as_deref(), Some("turn-1"));
        }
    }

    /// A resumed thread's first usage report is its replayed history: a baseline, billed nothing;
    /// the next bills its delta. A fresh thread's first report is billed.
    #[test]
    fn a_resumed_threads_first_usage_report_is_a_baseline() {
        let usage = |total: u64, last: u64| {
            json!({"method": "thread/tokenUsage/updated", "params": {"threadId": "t-1",
                "tokenUsage": {
                    "total": {"totalTokens": total, "inputTokens": total, "cachedInputTokens": 0,
                        "cacheWriteInputTokens": 0, "outputTokens": 0, "reasoningOutputTokens": 0},
                    "last": {"totalTokens": last, "inputTokens": last, "cachedInputTokens": 0,
                        "cacheWriteInputTokens": 0, "outputTokens": 0, "reasoningOutputTokens": 0},
                    "modelContextWindow": 1000}}})
        };
        let spend = |events: Vec<AgentEvent>| match events.into_iter().next() {
            Some(AgentEvent::UsageUpdate { spend, .. }) => spend.map(|s| s.total()),
            other => panic!("{other:?}"),
        };
        let (mut resumed, _) = mapper_with_turn(true);
        assert_eq!(spend(resumed.map(&usage(5000, 100))), None);
        assert_eq!(spend(resumed.map(&usage(5150, 150))), Some(150));
        let (mut fresh, _) = mapper_with_turn(false);
        assert_eq!(spend(fresh.map(&usage(100, 100))), Some(100));
    }

    /// A question's answers are not permissions: no unattended path may take one as "allow".
    #[test]
    fn question_answers_are_not_allowing_options() {
        let question = ask(
            "item/tool/requestUserInput",
            json!({"questions": [{"id": "q1", "question": "Which?", "isSecret": false,
                "options": [{"label": "A"}, {"label": "B"}]}]}),
        );
        let (_, options) = ask_prompt(&question).unwrap();
        assert!(options.iter().all(|o| !o.kind.allows()), "{options:?}");
    }

    /// Accepting an elicitation sends no form content, so only one that needs none is offered.
    #[test]
    fn an_elicitation_needing_content_is_refused() {
        let offered = |params: Value| ask_prompt(&ask("mcpServer/elicitation/request", params));
        assert!(offered(json!({"mode": "url", "url": "https://x", "message": "m"})).is_some());
        assert!(
            offered(json!({"mode": "form", "message": "m",
                "requestedSchema": {"type": "object", "properties": {}}}))
            .is_some()
        );
        assert!(
            offered(json!({"mode": "form", "message": "m",
                "requestedSchema": {"type": "object", "required": ["name"]}}))
            .is_none()
        );
        assert!(offered(json!({"mode": "openaiForm", "requestedSchema": {}})).is_none());
    }
}
