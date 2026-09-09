//! The Agent Client Protocol as a *client* — the [`super::IoBridge`]
//! implementation for any harness binary that is an ACP stdio endpoint.
//!
//! This is the inbound counterpart to [`super::acp`], which projects an
//! [`AgentEvent`] *onto* ACP's vocabulary. Here the traffic runs the other
//! way: `am` is the ACP **client**, the child process is the ACP **agent**,
//! and every `session/update` notification it pushes becomes an
//! [`AgentEvent`] by way of [`super::from_acp`] — the same mapping, not a
//! second copy of it.
//!
//! `_docs/references/acp-protocol.md` is the wire authority for everything
//! below; where this module and that document disagree, the document wins.
//!
//! Nothing here names a harness. An ACP endpoint is a binary that speaks
//! newline-delimited JSON-RPC 2.0 on its own stdin/stdout, and that is the
//! whole contract: which binary, which arguments and which credentials are
//! [`crate::harness`]'s business, at provisioning time.
//!
//! **This has not been pinned against a live ACP agent in this tree.** No
//! ACP-speaking harness binary is installed on the machine this was written
//! on, so every frame below is written to the spec reference rather than to a
//! capture. The first real session may correct it; the places most likely to
//! move are the ones flagged in prose here rather than silently guessed.
//!
//! This is **core** (always compiled, no feature gate): only `std::process`,
//! `std::sync`, `std::thread`, `std::fs`, `serde_json` and `tracing` are
//! used, matching [`super::jsonl`]'s and [`super::codex`]'s discipline.
//!
//! ## Design
//!
//! [`AcpBridge::new`] takes ownership of a spawned [`std::process::Child`]
//! (from [`super::spawn_piped`]), splits off stdin/stdout, spawns a dedicated
//! **reader thread** that owns stdout for the bridge's whole lifetime, and
//! only then runs the handshake — `initialize`, then `session/new` (or
//! `session/load` when resuming). The order matters: the handshake blocks on
//! responses, and a response can only arrive through a reader that is already
//! draining stdout.
//!
//! Every shared piece — the writer thread's queue, the pending-request map,
//! the outstanding permission table, the id counter, the session id, the live
//! turn's id, the session root, the agent's prompt capabilities and the event
//! sender — lives in one [`Shared`]
//! behind an `Arc`, held identically by the bridge, the reader thread and
//! every [`AcpInputSink`]. There is one such struct rather than eight
//! parameters because *four* parties touch those fields: the handshake, the
//! bridge's own [`IoBridge::send`], the reader thread (which answers the
//! agent's requests itself), and a detached sink handed out through
//! [`IoBridge::input`].
//!
//! ### Framing and multiplexing
//!
//! ACP is newline-delimited JSON — **one compact value per line, no
//! `Content-Length` headers** — and both peers are simultaneously client and
//! server, with independent id spaces. So a single line off stdout can be any
//! of four things, and the reader demultiplexes them by shape:
//!
//! | shape | meaning |
//! |---|---|
//! | `id` + (`result` \| `error`) | a response to a request **we** sent |
//! | `id` + `method` | an inbound **request** from the agent — we must reply |
//! | `method`, no `id` | a notification — never reply |
//!
//! The `result`/`error` test comes **first**, before the `method` test. It
//! has to: the two id spaces are independent, so the agent is free to send us
//! a request whose id collides with one of ours, and only the presence of a
//! `result` or `error` distinguishes "your answer" from "my question".
//!
//! ### Why a turn is not a blocking call
//!
//! `session/prompt` is a *long-lived* request: it returns only when the whole
//! turn ends, after every `session/update`, every permission round trip and
//! every tool call in between. Treating it like [`super::codex`]'s
//! `turn/start` ack — write, block, return — would park the caller's thread
//! for the length of the turn and, worse, park it behind work that needs that
//! same caller to answer a permission request.
//!
//! So [`AgentInput::Prompt`] allocates an id, records it in the shared turn
//! slot **before** writing the line, writes it, and returns immediately. The
//! reader recognises the response by that id, clears the slot and emits
//! [`AgentEvent::TurnEnded`] with the `stopReason` — which is exactly what a
//! consumer of this bridge already expects, since every other bridge reports
//! the end of a turn as an event too.
//!
//! That slot is one cell because ACP v1 is one turn at a time per session: a
//! [`AgentInput::Prompt`] sent while a turn is still on record is **refused**
//! with an error naming the id in flight, and no frame is written. Overwriting
//! the slot would strand the first turn's response with nothing to match, and
//! its caller would wait for a [`AgentEvent::TurnEnded`] that can never come.
//!
//! ### Request/response correlation
//!
//! Every outbound request that *does* block ([`initialize`], `session/new`,
//! `session/load`, `session/set_config_option`) takes a fresh id from an
//! `AtomicI64` and registers an `mpsc::Sender` in the shared pending map
//! **before** the line is written, so the reader can never observe the
//! response before someone is ready for it. The wait is always
//! `recv_timeout`, never a bare `recv`.
//!
//! `session/prompt` is the one request that is *not* in the pending map: its
//! id lives in the turn slot instead, and the reader checks that slot before
//! the map.
//!
//! ### Permissions are bidirectional
//!
//! This is the one place a bridge in this crate *answers* its agent. Every
//! NDJSON bridge reads a stream and writes prompts; here the agent sends
//! `session/request_permission` and **blocks** until we reply. That reply
//! therefore has to be reachable from the reader path: the reader records the
//! raw JSON-RPC id in the outstanding table, emits
//! [`AgentEvent::PermissionRequest`], and the answer arrives later as
//! [`AgentInput::AnswerPermission`] from whichever thread holds a sink. If
//! the answer instead sat behind the `session/prompt` response, it could
//! never come — the turn cannot end until the permission is answered.
//!
//! A cancel or a shutdown drains that table with `outcome: "cancelled"`
//! first, which the spec requires of a client and which also means the agent
//! is never left blocked on a question nobody will answer.
//!
//! The table's lifetime is the **turn's**, not the bridge's. When a turn ends
//! or the reader stops, whatever is left in it is dropped *without* a reply:
//! the agent has already retired those ids, and answering a retired id is the
//! protocol violation, not the silence. [`drop_outstanding`] and
//! [`cancel_outstanding`] are two drains with two different reasons.
//!
//! ### What we do not serve
//!
//! We advertise `fs.readTextFile`, `fs.writeTextFile` and
//! `session.configOptions.boolean`, and deliberately **not** `terminal` or
//! `elicitation`. Any inbound request outside what we advertised — the
//! `terminal/*` family, `elicitation/create`, `mcp/*`, any `_`-prefixed
//! vendor method — is answered `-32601 method not found`. That is
//! protocol-legal precisely because we never advertised the capability, and
//! silence is not: an unanswered request deadlocks the agent.
//!
//! The two `fs/*` methods we *do* serve are **confined to the session's
//! `cwd`**: an absolute path that does not resolve inside it is `-32602`,
//! with `..` resolved lexically before the comparison. `writeTextFile` is
//! advertised unconditionally, so nothing else stands between a
//! prompt-injected agent and `~/.ssh/authorized_keys`. See
//! [`absolute_param`], whose `NOTE` records the one limit: symlinks.
//!
//! ### Never hanging
//!
//! Six guards, the same shape as [`super::codex`]'s three:
//! 1. Every blocking wait uses `recv_timeout` — [`HANDSHAKE_TIMEOUT`] for
//!    the handshake, [`REQUEST_TIMEOUT`] for a mid-session request.
//! 2. `session/prompt` never blocks at all (above).
//! 3. **Nothing in this module ever writes to stdin.** A dedicated
//!    [`write_loop`] thread owns [`ChildStdin`] outright and every write is an
//!    enqueue onto its unbounded channel, so no caller can block under
//!    backpressure. This is not a refinement: the reader thread answers the
//!    agent's `fs/*` requests itself, and a reader blocked in `writeln!` on a
//!    full stdin pipe, while the agent blocks on a full stdout pipe nobody is
//!    draining, is a deadlock with no timeout on either side.
//! 4. **Every inbound request gets exactly one reply, on every path.** The
//!    agent blocks on its request with no timeout, so a `session/request_
//!    permission` this client cannot park — a poisoned table, a duplicate raw
//!    id, an event channel with no receiver — is answered `cancelled`
//!    immediately rather than left to a caller who will never see it.
//!    [`serve_request`] states the invariant.
//! 5. A [`ReaderExit`] guard runs when the reader stops, normally or on an
//!    unwind: it drops every pending `Sender` (so a waiter sees `Disconnected`
//!    at once instead of burning its timeout), forgets the outstanding
//!    permissions, and sends the explicit `None` that is the *only*
//!    end-of-stream signal — [`Shared`] holds a `Sender` clone for the
//!    bridge's whole life, so a dropped-sender EOF would never arrive.
//! 6. [`Drop`] closes stdin (a message to the writer thread, not a lock —
//!    teardown acquires no mutex at all), bound-waits for the child with
//!    `try_wait`, kills and reaps it on expiry, then joins both threads.
//!
//! ## Logging
//!
//! Every raw line, in either direction, is a `trace!`; every mapped event is
//! a `debug!`. Raw frames carry prompts and file contents, which is why they
//! sit a level below everything else: an embedder's default filter collects
//! `debug` and leaves them out until someone asks for them by name. Races — a
//! cancel for a turn that already ended, an answer for a permission that
//! vanished — are `debug!` too, not returned errors: the caller asked for a
//! state the session is already in.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{
    AgentEvent, AgentInput, AgentInputSink, ConfigSetting, Content, IoBridge, Origin,
    PermissionKind, PermissionOption, PermissionOutcome, Spend, StopReason, ToolCall,
    ToolCallUpdate, ToolKind, ToolStatus,
};

/// How long the handshake waits for `initialize` / `session/new` /
/// `session/load`. Generous, because a cold agent may be starting a language
/// server or reading a large config — but bounded, because a silently hung
/// agent must not hang the caller forever.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a mid-session blocking request (`session/set_config_option`)
/// waits for its response. Shorter than the handshake: the session is already
/// up, so a slow answer here is a fault rather than a cold start.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// How long [`Drop`] waits for the child to exit after closing stdin before
/// killing it, matching [`super::jsonl`] and [`super::codex`].
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// Pending outbound requests awaiting a response, keyed by the `id` we sent.
/// `session/prompt` is deliberately absent — see [`TurnSlot`].
type PendingMap = Arc<Mutex<HashMap<i64, mpsc::Sender<Value>>>>;

/// The agent's `session/request_permission` requests we have emitted and not
/// yet answered: our `request_id` string → the raw JSON-RPC `id` `Value` the
/// reply must echo (a number or a string, whichever the agent used).
///
/// Shared between the reader thread (which records an ask) and every input
/// sink (which answers one, or cancels all of them on a cancel or shutdown).
type Outstanding = Arc<Mutex<HashMap<String, Value>>>;

/// The live turn's `session/prompt` id, shared between the writer that
/// allocated it and the reader that will see its response. `None` between
/// turns.
type TurnSlot = Arc<Mutex<Option<i64>>>;

/// One event, and the stdout line that produced it where there was one.
///
/// A line can map to several events, so the line is shared rather than copied
/// — each event carries a handle onto the same text. An event this bridge
/// synthesizes locally (the handshake's [`AgentEvent::SessionStarted`]) has no
/// line at all.
type Framed = (AgentEvent, Option<Arc<str>>);

/// Which content blocks the agent said it accepts in a prompt.
///
/// `text` and `resource_link` are unconditional — the spec requires every
/// agent to accept both — so only the three gated kinds are recorded. An
/// agent that does not advertise a kind gets that content down-converted to
/// text rather than a frame it is entitled to reject.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PromptCaps {
    image: bool,
    audio: bool,
    embedded_context: bool,
}

/// What the reader thread has to remember across notifications, because ACP
/// does not carry it on the wire.
///
/// All three fields exist for the same reason: [`super::from_acp`] is a pure
/// per-notification mapping, and these are the pieces of the
/// [`AgentEvent::UsageUpdate`] and [`Origin`] contracts that only a stateful
/// reader can honour.
#[derive(Debug, Default)]
struct ReaderState {
    /// Open `Task`/`Agent` tool calls, oldest first: `toolCallId` and the
    /// subagent type its `rawInput` named.
    delegates: Vec<(String, Option<String>)>,
    /// Live subagent sessions from the ACP subagent extension (draft PR
    /// #1992): the child's `sessionId` and the `name` it was spawned under.
    /// Exact attribution, where `delegates` only guesses.
    subagents: HashMap<String, String>,
    /// The last `cost.amount` seen, per session id — ACP states it as a
    /// session-cumulative figure, subtracted to get the per-report delta the
    /// event contracts for. Keyed because a subagent session reports its own
    /// window and must not move the parent's memo.
    cost_totals: HashMap<String, f64>,
    /// The `category: "model"` config option's current value, so a usage
    /// report can name a model ACP never puts on the report itself.
    model: Option<String>,
    /// The last `used`/`size` a `usage_update` reported, so the turn's own
    /// spend report can state the same occupancy rather than reading as a
    /// context window that just emptied. `(0, 0)` until one arrives — which
    /// for an agent that sends no `usage_update` at all (Grok) is the honest
    /// answer, it names no window anywhere.
    occupancy: (u64, u64),
}

/// Everything the bridge, its reader thread and every [`AcpInputSink`] share.
///
/// One struct rather than a parameter list because all four writers need all
/// of it, and because a field added here cannot then be forgotten in one of
/// the call sites.
struct Shared {
    /// The writer thread's queue. `Some(value)` is one frame to write;
    /// `None` closes stdin. Nobody but the writer thread ever touches
    /// [`ChildStdin`], so no caller — least of all the reader thread — can
    /// ever block in a write. See [`write_loop`].
    writes: mpsc::Sender<Option<Value>>,
    pending: PendingMap,
    outstanding: Outstanding,
    next_id: Arc<AtomicI64>,
    /// The agent's own id for the conversation, from `session/new`'s result
    /// (or the id we resumed). Written once, at the end of the handshake.
    session_id: Arc<OnceLock<String>>,
    turn: TurnSlot,
    /// Written once, from `initialize`'s `agentCapabilities`.
    prompt_caps: Arc<OnceLock<PromptCaps>>,
    /// The session's `cwd`, absolute and lexically normalized: the one
    /// directory tree an `fs/*` request may name. See [`absolute_param`].
    root: PathBuf,
    /// A handle onto the reader thread's channel, so the handshake and a
    /// `session/set_config_option` can push a locally-derived event.
    tx: mpsc::Sender<Option<Framed>>,
    /// See [`ReaderState`]. Written by the reader thread and by the handshake.
    state: Mutex<ReaderState>,
}

/// A live bridge to a child process speaking ACP v1 over newline-delimited
/// JSON-RPC 2.0 on stdin/stdout.
pub struct AcpBridge {
    child: Child,
    events: mpsc::Receiver<Option<Framed>>,
    reader: Option<std::thread::JoinHandle<()>>,
    writer: Option<std::thread::JoinHandle<()>>,
    shared: Arc<Shared>,
    /// The agent's `agentCapabilities`, verbatim from `initialize` — kept so a
    /// caller can ask what this endpoint can do without a second handshake.
    /// `loadSession` and `promptCapabilities` are the two this bridge itself
    /// reads.
    agent_capabilities: Value,
}

/// The detached input side of an [`AcpBridge`], for a caller pumping events on
/// one thread and prompting, cancelling or answering permissions from
/// another. See [`AgentInputSink`].
pub struct AcpInputSink {
    shared: Arc<Shared>,
}

impl AgentInputSink for AcpInputSink {
    fn send(&self, input: AgentInput) -> crate::Result<()> {
        write_input(&self.shared, input)
    }
}

impl AcpBridge {
    /// Drive an ACP stdio endpoint over the pipes of an already-spawned child
    /// (piped stdin/stdout, e.g. from [`super::spawn_piped`]), running the
    /// whole handshake synchronously: `initialize`, then `session/new` — or
    /// `session/load` when `resume` names a session and the agent advertises
    /// `loadSession`.
    ///
    /// `cwd` becomes the session's `cwd`, which ACP requires to be absolute;
    /// a relative path is resolved against the process's working directory.
    ///
    /// Errors (without hanging — every step is timeout-bounded) if:
    /// - `child`'s stdin/stdout are not piped (a programmer error —
    ///   [`super::spawn_piped`] always pipes both);
    /// - `cwd` cannot be made absolute;
    /// - the agent answers a protocol version other than 1;
    /// - `resume` was asked for but `agentCapabilities.loadSession` is absent;
    /// - any handshake request times out or comes back a JSON-RPC `error`;
    /// - `session/new`'s response is missing `sessionId`.
    ///
    /// On any handshake error the partially-built bridge — reader thread and
    /// child process included — is torn down by [`Drop`] as the function
    /// returns.
    pub fn new(mut child: Child, cwd: &Path, resume: Option<&str>) -> crate::Result<Self> {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("child stdin is not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("child stdout is not piped"))?;

        let root = absolute_cwd(cwd)?;
        let cwd = root.display().to_string();

        let (tx, rx) = mpsc::channel();
        // Writes go through a queue owned by their own thread, so a write can
        // never block the thread that is draining stdout — see [`write_loop`].
        let (writes, write_rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            writes,
            pending: Arc::new(Mutex::new(HashMap::new())),
            outstanding: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicI64::new(1)),
            session_id: Arc::new(OnceLock::new()),
            turn: Arc::new(Mutex::new(None)),
            prompt_caps: Arc::new(OnceLock::new()),
            root,
            tx,
            state: Mutex::new(ReaderState::default()),
        });

        let writer = std::thread::spawn(move || write_loop(stdin, write_rx));
        // The reader has to be draining stdout before the first blocking
        // request, or its response could never arrive.
        let reader_shared = Arc::clone(&shared);
        let reader = std::thread::spawn(move || read_loop(stdout, reader_shared));

        let mut bridge = Self {
            child,
            events: rx,
            reader: Some(reader),
            writer: Some(writer),
            shared,
            agent_capabilities: Value::Null,
        };

        bridge.handshake(&cwd, resume)?;

        Ok(bridge)
    }

    /// `initialize` → `session/new` (or `session/load`), recording the agent's
    /// capabilities and session id and emitting the two events the result
    /// implies.
    fn handshake(&mut self, cwd: &str, resume: Option<&str>) -> crate::Result<()> {
        let init = rpc_request(
            &self.shared,
            "initialize",
            initialize_params(),
            HANDSHAKE_TIMEOUT,
        )?;

        let version = init.get("protocolVersion").and_then(Value::as_i64);
        if version != Some(1) {
            anyhow::bail!(
                "acp agent answered protocol version {} but this client speaks version 1",
                version.map_or_else(|| "none".to_string(), |v| v.to_string())
            );
        }

        self.agent_capabilities = init
            .get("agentCapabilities")
            .cloned()
            .unwrap_or(Value::Null);
        let _ = self
            .shared
            .prompt_caps
            .set(prompt_caps(&self.agent_capabilities));
        let load_session = self
            .agent_capabilities
            .get("loadSession")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // Authentication is *not* attempted. This bridge cannot drive an
        // interactive or OAuth login, and `am` settles credentials at
        // provisioning time (`crate::harness`), so a non-empty `authMethods`
        // is information rather than a step. If auth really is required the
        // agent rejects `session/new` with -32000, and that surfaces below as
        // a `new()` failure naming the methods it advertised.
        let auth_methods = auth_method_ids(&init);
        if !auth_methods.is_empty() {
            tracing::debug!(
                methods = ?auth_methods,
                "acp agent advertises auth methods; not attempting `authenticate`"
            );
        }

        let (method, params) = match resume {
            Some(session_id) => {
                if !load_session {
                    anyhow::bail!(
                        "cannot resume acp session '{session_id}': the agent does not advertise \
                         `agentCapabilities.loadSession`"
                    );
                }
                (
                    "session/load",
                    // `mcpServers` is always `[]` and never null: `am` injects
                    // MCP servers through the harness's own config files at
                    // provisioning time, not over the wire.
                    json!({"sessionId": session_id, "cwd": cwd, "mcpServers": []}),
                )
            }
            // Same rule for a fresh session: `[]`, never null.
            None => ("session/new", json!({"cwd": cwd, "mcpServers": []})),
        };

        let result = match rpc_request(&self.shared, method, params, HANDSHAKE_TIMEOUT) {
            Ok(result) => result,
            Err(error) => {
                // -32000 is ACP's "auth required". Name the method ids the
                // agent advertised, or the failure is undiagnosable.
                if error.to_string().contains("-32000") && !auth_methods.is_empty() {
                    anyhow::bail!(
                        "{error}; the agent advertised auth methods {auth_methods:?} and this \
                         client cannot drive a login — provision credentials for the harness first"
                    );
                }
                return Err(error);
            }
        };

        let session_id = match resume {
            // `session/load`'s result carries no `sessionId` — the client
            // already supplied it.
            Some(session_id) => session_id.to_string(),
            None => result
                .get("sessionId")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("session/new response missing sessionId"))?
                .to_string(),
        };
        let _ = self.shared.session_id.set(session_id.clone());

        // NOTE: `session/load` replays the whole conversation as
        // `session/update` notifications *before* it responds. Nothing here
        // suppresses that, and nothing should: a consumer rebuilding a
        // transcript wants exactly those events. It does mean a resumed
        // conversation re-emits its history, in order, ahead of the
        // `SessionStarted` below.
        for ev in session_events(&session_id, &result) {
            if let AgentEvent::SessionStarted { model, .. } = &ev
                && let Ok(mut state) = self.shared.state.lock()
            {
                state.model.clone_from(model);
            }
            emit(&self.shared.tx, ev, None);
        }

        Ok(())
    }

    /// The agent's `agentCapabilities`, verbatim from `initialize`.
    pub fn agent_capabilities(&self) -> &Value {
        &self.agent_capabilities
    }
}

impl IoBridge for AcpBridge {
    fn send(&mut self, input: AgentInput) -> crate::Result<()> {
        write_input(&self.shared, input)
    }

    fn next_event(&mut self) -> crate::Result<Option<AgentEvent>> {
        Ok(self.next_event_raw()?.map(|(event, _)| event))
    }

    fn next_event_raw(&mut self) -> crate::Result<Option<(AgentEvent, Option<String>)>> {
        match self.events.recv() {
            Ok(Some((ev, raw))) => Ok(Some((ev, raw.map(|line| line.to_string())))),
            // The reader thread's explicit "done", and the *only* EOF signal
            // there is: [`Shared`] holds a `Sender` clone for as long as the
            // bridge lives, so this channel cannot disconnect underneath us.
            // That is why the sentinel is sent from a [`ReaderExit`] guard's
            // `Drop` — it has to survive a panicking reader too.
            Ok(None) => Ok(None),
            // Unreachable while the bridge holds `Shared` (above); belt and
            // braces, and as honest an EOF as the explicit signal.
            Err(mpsc::RecvError) => Ok(None),
        }
    }

    fn input(&self) -> Option<Arc<dyn AgentInputSink>> {
        Some(Arc::new(AcpInputSink {
            shared: Arc::clone(&self.shared),
        }))
    }

    /// Kill-by-pid over the child this bridge owns; see [`crate::io::ProcessKill`].
    fn killer(&self) -> Option<Arc<dyn crate::io::AgentKill>> {
        Some(Arc::new(crate::io::ProcessKill::new(&self.child)))
    }
}

impl Drop for AcpBridge {
    fn drop(&mut self) {
        // Close stdin first (best-effort "please stop"), then give the child a
        // bounded window to drain and exit before killing it.
        //
        // This acquires **no mutex**, and nothing below does either: teardown
        // must not be able to wait on a lock some other thread holds. Closing
        // stdin is now a message to the writer thread rather than a field to
        // clear under a lock.
        close_stdin(&self.shared);

        let deadline = Instant::now() + DRAIN_TIMEOUT;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        // NOTE: this kills only the direct child, not its
                        // process group — that needs a `killpg` syscall
                        // wrapper, and this crate is `#![forbid(unsafe_code)]`
                        // with no libc dependency to provide one. Same
                        // limitation as [`super::codex`].
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
        // the reader thread's scan loop; the writer thread has already seen
        // its `None` (or the channel disconnect behind it).
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

// ── the handshake's frames ─────────────────────────────────────────────

/// `initialize`'s params.
///
/// `protocolVersion` is a bare **integer**, not a string — the one field of
/// this frame most likely to be typed wrong. `terminal` and `elicitation` are
/// deliberately absent: see the module docs' "What we do not serve".
fn initialize_params() -> Value {
    json!({
        "protocolVersion": 1,
        "clientInfo": {
            "name": "agent-manager",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "clientCapabilities": {
            "fs": {"readTextFile": true, "writeTextFile": true},
            "session": {"configOptions": {"boolean": {}}},
            // The subagent extension (ACP draft PR #1992) is gated on this,
            // which the adapters carry under their `jetbrains.air` vendor
            // namespace until the draft lands in ACP proper. Without it a
            // delegate arrives as a plain tool call and [`attribute`]'s
            // heuristic is all there is.
            "_meta": {"jetbrains": {"air": {
                "version": 1,
                "capabilities": ["nativeSubagentSessions"],
            }}},
        },
    })
}

/// The `authMethods` ids the agent advertised, or empty if it advertised none.
fn auth_method_ids(init: &Value) -> Vec<String> {
    init.get("authMethods")
        .and_then(Value::as_array)
        .map(|methods| {
            methods
                .iter()
                .filter_map(|method| method.get("id").and_then(Value::as_str))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Which gated prompt content kinds the agent accepts.
fn prompt_caps(agent_capabilities: &Value) -> PromptCaps {
    let caps = agent_capabilities.get("promptCapabilities");
    let flag = |key: &str| {
        caps.and_then(|c| c.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    PromptCaps {
        image: flag("image"),
        audio: flag("audio"),
        embedded_context: flag("embeddedContext"),
    }
}

/// ACP requires an absolute `cwd`. Resolve a relative one against the
/// process's working directory and refuse anything that still is not
/// absolute.
///
/// Deliberately **not** `canonicalize`: that resolves symlinks, so the
/// session's `cwd` would differ textually from the directory the process was
/// actually spawned in (`/tmp` → `/private/tmp` on macOS, any symlinked
/// repository root), and the agent would then report paths under a root the
/// host never named. [`std::path::absolute`] makes a path absolute without
/// touching the filesystem, which is exactly the job.
fn absolute_cwd(cwd: &Path) -> crate::Result<PathBuf> {
    let absolute = std::path::absolute(cwd)
        .map_err(|err| anyhow::anyhow!("cannot resolve the cwd '{cwd:?}': {err}"))?;
    if !absolute.is_absolute() {
        anyhow::bail!("acp requires an absolute cwd; '{cwd:?}' could not be made one");
    }
    Ok(lexically_normal(&absolute))
}

/// `path` with every `.` dropped and every `..` resolved **textually**, no
/// filesystem access at all.
///
/// This is what the `fs/*` confinement check compares, and it has to be
/// lexical: the target of a `fs/write_text_file` need not exist yet, so
/// `canonicalize` cannot be used, and resolving `..` after the prefix check
/// would let `<root>/../etc/passwd` through.
fn lexically_normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The events `session/new`'s (or `session/load`'s) result implies.
///
/// `tools` and `agents` are always empty: ACP reports neither at session
/// setup — a tool call is only named when it happens, and there is no
/// subagent vocabulary at all — so guessing would be worse than saying
/// nothing.
fn session_events(session_id: &str, result: &Value) -> Vec<AgentEvent> {
    let config = result.get("configOptions").filter(|c| !c.is_null());
    // The mode lives in one of two places: the legacy `modes` block, or a
    // config option with `category: "mode"`. §10 of the reference calls
    // config options the successor and `modes` the deprecated sibling, but an
    // agent may still send only the latter.
    let mode = result
        .get("modes")
        .and_then(|modes| modes.get("currentModeId"))
        .and_then(Value::as_str)
        .map(String::from)
        .or_else(|| config.and_then(|c| config_current_value(c, "mode")));
    // ACP has no model field anywhere; the model is a config option with
    // `category: "model"`, and its `currentValue` is the model id.
    let model = config.and_then(|c| config_current_value(c, "model"));

    let mut events = vec![AgentEvent::SessionStarted {
        session_id: Some(session_id.to_string()),
        model,
        mode,
        tools: Vec::new(),
        agents: Vec::new(),
    }];
    if let Some(config) = config
        && let Some(ev) = config_option_update(config)
    {
        events.push(ev);
    }
    events
}

/// The `currentValue` of the first config option in `category`, where it is a
/// string. A boolean option in these categories would not name a mode or a
/// model, so it is passed over rather than stringified.
fn config_current_value(options: &Value, category: &str) -> Option<String> {
    options
        .as_array()?
        .iter()
        .find(|option| option.get("category").and_then(Value::as_str) == Some(category))
        .and_then(|option| option.get("currentValue"))
        .and_then(Value::as_str)
        .map(String::from)
}

/// An [`AgentEvent::ConfigOptionUpdate`] from a bare `configOptions` array, by
/// feeding a synthesized `config_option_update` session update through
/// [`super::from_acp`] — the option mapping lives there and is not duplicated
/// here.
fn config_option_update(config_options: &Value) -> Option<AgentEvent> {
    super::from_acp(&json!({
        "sessionUpdate": "config_option_update",
        "configOptions": config_options,
    }))
}

// ── writing ────────────────────────────────────────────────────────────

/// Turn one [`AgentInput`] into the JSON-RPC call(s) it implies, and send
/// them.
///
/// Shared by [`IoBridge::send`] and [`AcpInputSink::send`] so the two cannot
/// drift apart — a prompt sent from a pump thread has to reach the agent in
/// exactly the same shape as one sent from the owner's.
fn write_input(shared: &Shared, input: AgentInput) -> crate::Result<()> {
    let session_id = shared.session_id.get().cloned().unwrap_or_default();

    match input {
        AgentInput::Prompt { ref content } => {
            // `session/prompt` only returns when the turn is over, so this
            // must not block: allocate the id, record it so the reader can
            // recognise the eventual response, write, return.
            let id = {
                let mut slot = shared
                    .turn
                    .lock()
                    .map_err(|_| anyhow::anyhow!("acp bridge turn-slot lock poisoned"))?;
                // ACP v1 is one turn at a time per session, and the turn slot
                // is one cell: a second prompt would overwrite the first id,
                // the first response would then match nothing, and the caller
                // would wait for a `TurnEnded` that can never come. An honest
                // error beats a lost turn.
                if let Some(previous) = *slot {
                    anyhow::bail!(
                        "acp runs one turn at a time: the prompt with id {previous} is still in \
                         flight — wait for `TurnEnded` or cancel it first"
                    );
                }
                let id = shared.next_id.fetch_add(1, Ordering::SeqCst);
                // Recorded *before* the write, so the reader can never see
                // the response before the slot names it.
                *slot = Some(id);
                id
            };
            let caps = shared.prompt_caps.get().copied().unwrap_or_default();
            let params = json!({
                "sessionId": session_id,
                "prompt": prompt_blocks(content, caps),
            });
            let line =
                json!({"jsonrpc": "2.0", "id": id, "method": "session/prompt", "params": params});
            if let Err(err) = write_line(shared, line) {
                // Only a failed *enqueue* un-records the turn: a successful
                // one means the agent will receive the prompt and answer it.
                // And only if the slot still holds *our* id — another thread
                // may already have started the next turn.
                if let Ok(mut slot) = shared.turn.lock()
                    && *slot == Some(id)
                {
                    *slot = None;
                }
                return Err(err);
            }
            Ok(())
        }

        AgentInput::AnswerPermission {
            request_id,
            outcome,
            updated_input,
        } => {
            if updated_input.is_some() {
                // ACP's `RequestPermissionResponse` carries an outcome and
                // nothing else — there is no field to rewrite the tool's
                // input with, so a caller's rewrite cannot be honoured.
                tracing::debug!(
                    %request_id,
                    "acp has no field for `updated_input`; the tool runs with what it asked for"
                );
            }
            // Answering retires the ask, so a later cancel does not answer it
            // a second time.
            let id = shared
                .outstanding
                .lock()
                .ok()
                .and_then(|mut table| table.remove(&request_id));
            let Some(id) = id else {
                // The turn may simply have ended first — a race, not a fault.
                tracing::debug!(%request_id, "acp answer for an unknown permission request");
                return Ok(());
            };
            write_line(shared, permission_reply(&id, &outcome))
        }

        AgentInput::Cancel => {
            // Order is the spec's: every pending permission is answered
            // `cancelled` *first*, then the notification goes out. An agent
            // left blocked on a question cannot act on the cancel.
            cancel_outstanding(shared);
            // A notification — no id, and no response ever comes, so nothing
            // waits for one. The reader keeps going: the agent may send more
            // updates and MUST then end the turn with
            // `stopReason: "cancelled"` rather than a JSON-RPC error.
            write_line(shared, cancel_notification(&session_id))
        }

        AgentInput::Shutdown => {
            cancel_outstanding(shared);
            // Teardown: closing stdin is the EOF the agent exits on, and the
            // reader keeps draining stdout until it actually does. The
            // `cancelled` replies above were enqueued first, so the writer
            // thread flushes them before it drops stdin.
            close_stdin(shared);
            Ok(())
        }

        AgentInput::SetConfigOption { config_id, value } => {
            let result = rpc_request(
                shared,
                "session/set_config_option",
                set_config_params(&session_id, &config_id, &value),
                REQUEST_TIMEOUT,
            )?;
            // The result is always the *complete* option set with current
            // values, since setting one option can change another — so it is
            // exactly a `ConfigOptionUpdate`, built through the same mapping
            // as the notification form.
            if let Some(options) = result.get("configOptions")
                && let Some(ev) = config_option_update(options)
            {
                emit(&shared.tx, ev, None);
            }
            Ok(())
        }
    }
}

/// `session/cancel` — a **notification**, so no `id` key at all.
fn cancel_notification(session_id: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/cancel",
        "params": {"sessionId": session_id},
    })
}

/// `session/set_config_option`'s params.
///
/// `type: "boolean"` is added **only** for a boolean value: the string form is
/// the default variant and must not carry a `type` at all.
fn set_config_params(session_id: &str, config_id: &str, value: &ConfigSetting) -> Value {
    match value {
        ConfigSetting::Text(text) => json!({
            "sessionId": session_id,
            "configId": config_id,
            "value": text,
        }),
        ConfigSetting::Flag(flag) => json!({
            "sessionId": session_id,
            "configId": config_id,
            "value": flag,
            "type": "boolean",
        }),
    }
}

/// The prompt's content blocks, in ACP's `ContentBlock` shape.
///
/// `text` and `resource_link` go through unchanged — every agent must accept
/// both. The three gated kinds are down-converted to text where the agent did
/// not advertise them, because adapting content the agent cannot accept is
/// the *client's* job per §8 of the reference, and sending it anyway earns a
/// rejected turn.
fn prompt_blocks(content: &[Content], caps: PromptCaps) -> Vec<Value> {
    content
        .iter()
        .map(|block| content_block(block, caps))
        .collect()
}

fn content_block(block: &Content, caps: PromptCaps) -> Value {
    match block {
        Content::Text { text } => json!({"type": "text", "text": text}),

        Content::Image {
            data,
            mime_type,
            uri,
        } => {
            if !caps.image {
                return omitted("image", mime_type);
            }
            let mut value = json!({"type": "image", "data": data, "mimeType": mime_type});
            if let Some(uri) = uri {
                value["uri"] = json!(uri);
            }
            value
        }

        Content::Audio { data, mime_type } => {
            if !caps.audio {
                return omitted("audio", mime_type);
            }
            json!({"type": "audio", "data": data, "mimeType": mime_type})
        }

        Content::ResourceLink {
            uri,
            name,
            mime_type,
            title,
            description,
            size,
        } => {
            let mut value = json!({"type": "resource_link", "uri": uri, "name": name});
            if let Some(mime_type) = mime_type {
                value["mimeType"] = json!(mime_type);
            }
            if let Some(title) = title {
                value["title"] = json!(title);
            }
            if let Some(description) = description {
                value["description"] = json!(description);
            }
            if let Some(size) = size {
                value["size"] = json!(size);
            }
            value
        }

        Content::Resource { resource } => {
            let (uri, body, mime_type) = match resource {
                super::ResourceContents::Text {
                    uri,
                    text,
                    mime_type,
                } => (uri, json!({"text": text}), mime_type),
                super::ResourceContents::Blob {
                    uri,
                    blob,
                    mime_type,
                } => (uri, json!({"blob": blob}), mime_type),
            };
            if !caps.embedded_context {
                // A link is the honest down-conversion: the agent can still
                // fetch it, and every agent must accept `resource_link`.
                return json!({"type": "resource_link", "uri": uri, "name": uri});
            }
            let mut inner = json!({"uri": uri});
            if let Value::Object(body) = body {
                for (key, val) in body {
                    inner[key] = val;
                }
            }
            if let Some(mime_type) = mime_type {
                inner["mimeType"] = json!(mime_type);
            }
            json!({"type": "resource", "resource": inner})
        }
    }
}

/// The text block that stands in for content the agent said it cannot accept.
/// Naming what was dropped beats silently sending nothing.
fn omitted(kind: &str, mime_type: &str) -> Value {
    json!({
        "type": "text",
        "text": format!("[{kind} content ({mime_type}) omitted: the agent does not accept it]"),
    })
}

/// Answer every outstanding permission request `cancelled`, and forget them —
/// an ask answered once must not be answered twice. Shared by
/// [`AgentInput::Cancel`] and [`AgentInput::Shutdown`]: both leave the turn,
/// and an agent holding an unanswered ask has no timeout to fall back on.
fn cancel_outstanding(shared: &Shared) {
    for id in take_outstanding(shared) {
        let _ = write_line(shared, permission_reply(&id, &PermissionOutcome::Cancelled));
    }
}

/// Forget every outstanding permission request **without answering it**.
///
/// This is *not* [`cancel_outstanding`] with the writes elided, and the two
/// must not be merged. The distinction is whose ask it is:
///
/// - On a [`AgentInput::Cancel`] or [`AgentInput::Shutdown`] the agent is
///   still blocked on the question, so every ask gets its `cancelled` reply —
///   that is [`cancel_outstanding`].
/// - When a **turn ends** the agent retired those ids itself (it is entitled
///   to end a turn `cancelled` or `refusal` without waiting for an answer),
///   and when the **reader exits** the agent is gone. Writing a `result` for
///   a retired id is a protocol violation a strict peer may drop the
///   connection over, so these entries are simply dropped. Leaving them would
///   grow the table for the bridge's whole life and let a later `Cancel` in a
///   different turn answer an id from this one.
fn drop_outstanding(shared: &Shared, reason: &str) {
    let dropped = take_outstanding(shared).len();
    if dropped > 0 {
        tracing::debug!(dropped, reason, "acp outstanding permissions dropped");
    }
}

/// Empty the outstanding table and hand back the raw ids it held. Tolerates a
/// poisoned lock: this runs on teardown paths that must not give up.
fn take_outstanding(shared: &Shared) -> Vec<Value> {
    let mut table = match shared.outstanding.lock() {
        Ok(table) => table,
        Err(poisoned) => poisoned.into_inner(),
    };
    table.drain().map(|(_, id)| id).collect()
}

/// The writer thread body: the **only** owner of [`ChildStdin`].
///
/// Every write in this module is an enqueue onto this thread's channel, which
/// is why no caller can ever block in a write. That matters most for the
/// reader thread, which answers the agent's `fs/*` requests itself: if it
/// wrote inline it could block on a full stdin pipe while the agent blocked on
/// a full stdout pipe waiting for the reader to drain it — a deadlock with no
/// timeout and no misbehaviour required.
///
/// Exits on `Some` write failure (dropping stdin, so the agent sees the EOF
/// that is the honest outcome), on the explicit `None`, and on a channel
/// disconnect.
fn write_loop(mut stdin: ChildStdin, writes: mpsc::Receiver<Option<Value>>) {
    while let Ok(Some(value)) = writes.recv() {
        if let Err(err) = writeln!(stdin, "{value}").and_then(|()| stdin.flush()) {
            tracing::debug!(%err, "acp stdin write failed; closing stdin");
            return;
        }
    }
    // `Ok(None)` — an explicit close — or a disconnect: drop stdin and stop.
}

/// Enqueue `value` as one newline-delimited JSON-RPC line for the writer
/// thread. Never blocks: the queue is unbounded, so backpressure on the
/// child's stdin cannot reach the caller.
///
/// `Err` means only that the writer thread is gone — stdin was closed by
/// [`AgentInput::Shutdown`], by [`Drop`], or by a write failure — so the frame
/// will never be written.
fn write_line(shared: &Shared, value: Value) -> crate::Result<()> {
    // Logged here rather than in the writer thread, so the order in the log is
    // the order of intent.
    tracing::trace!(direction = "out", frame = %value, "acp jsonrpc");
    shared
        .writes
        .send(Some(value))
        .map_err(|_| anyhow::anyhow!("the acp bridge's stdin is closed"))
}

/// Tell the writer thread to drop stdin and exit — the EOF an agent stops on.
/// Acquires no lock, so [`Drop`] can always make progress.
fn close_stdin(shared: &Shared) {
    let _ = shared.writes.send(None);
}

/// Send a JSON-RPC *request* and block (bounded by `timeout`) for its matching
/// response, returning its `result`.
///
/// Registers the id in the pending map before writing the line, so the reader
/// thread can never observe (and drop) the response before this call is ready
/// for it. Errors on a JSON-RPC `error`, a timeout, or stdout closing — never
/// blocks forever.
fn rpc_request(
    shared: &Shared,
    method: &str,
    params: Value,
    timeout: Duration,
) -> crate::Result<Value> {
    let id = shared.next_id.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = mpsc::channel();
    {
        let mut pending = shared
            .pending
            .lock()
            .map_err(|_| anyhow::anyhow!("acp bridge pending-map lock poisoned"))?;
        pending.insert(id, tx);
    }

    // Registered above, enqueued here, in that order: the queue is FIFO and
    // the writer thread is prompt, so the response cannot outrun the entry.
    let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    if let Err(err) = write_line(shared, line) {
        if let Ok(mut pending) = shared.pending.lock() {
            pending.remove(&id);
        }
        return Err(err);
    }

    match rx.recv_timeout(timeout) {
        Ok(response) => {
            if let Some(error) = response.get("error") {
                anyhow::bail!("acp agent returned an error for `{method}`: {error}");
            }
            Ok(response.get("result").cloned().unwrap_or(Value::Null))
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            if let Ok(mut pending) = shared.pending.lock() {
                pending.remove(&id);
            }
            anyhow::bail!("timed out after {timeout:?} waiting for a response to `{method}`")
        }
        // Reachable, and the fast path for a dead agent: the reader drops
        // every pending `Sender` as it exits (see [`ReaderExit`]), so a
        // waiter learns at once instead of burning the whole timeout.
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            anyhow::bail!("the acp agent's stdout closed before a response to `{method}` arrived")
        }
    }
}

// ── reading ────────────────────────────────────────────────────────────

/// Everything the reader owes the rest of the bridge when it stops, in a
/// guard so it happens on a normal return *and* on an unwind.
///
/// A guard rather than `catch_unwind`: it needs no `AssertUnwindSafe`, and its
/// `Drop` runs exactly once, which is what "the sentinel is sent exactly once"
/// requires.
struct ReaderExit(Arc<Shared>);

impl Drop for ReaderExit {
    fn drop(&mut self) {
        // Nobody will ever answer a pending request now. Dropping the
        // `Sender`s makes every `recv_timeout` see `Disconnected` at once
        // rather than burn its whole timeout — an agent that dies right after
        // `initialize` must not cost the caller `HANDSHAKE_TIMEOUT`.
        let mut pending = match self.0.pending.lock() {
            Ok(pending) => pending,
            Err(poisoned) => poisoned.into_inner(),
        };
        pending.clear();
        drop(pending);
        // The agent is gone: it is not waiting for these and could not read a
        // reply anyway. Dropped, not answered — see [`drop_outstanding`].
        drop_outstanding(&self.0, "the acp reader exited");
        // The one and only end-of-stream signal; see [`IoBridge::next_event`].
        let _ = self.0.tx.send(None);
    }
}

/// The reader thread body. Ends with the explicit `None` that tells
/// [`IoBridge::next_event`] the stream is really over — [`Shared`] keeps a
/// `Sender` clone alive for the bridge's whole life, so a dropped-sender EOF
/// would never arrive.
fn read_loop(stdout: ChildStdout, shared: Arc<Shared>) {
    let _exit = ReaderExit(Arc::clone(&shared));
    read_stream(stdout, &shared);
}

/// Scan stdout line by line and route each parsed frame by shape — see the
/// module docs' "Framing and multiplexing". Returns on EOF, a channel
/// disconnect (nobody left to receive), or a poisoned lock.
fn read_stream(stdout: ChildStdout, shared: &Shared) {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        tracing::trace!(direction = "in", frame = %line, "acp jsonrpc");
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            // Not JSON — skipped, never an error: a malformed frame must not
            // fail the conversation.
            continue;
        };
        let raw: Arc<str> = Arc::from(line);

        let id = value.get("id").cloned().filter(|id| !id.is_null());
        let method = value.get("method").and_then(Value::as_str);
        let is_response = value.get("result").is_some() || value.get("error").is_some();

        let keep_reading = match (id, method) {
            // A response to one of OUR requests. Checked before `method`
            // because the two id spaces are independent: only the presence of
            // a `result`/`error` tells an answer from a question.
            (Some(id), _) if is_response => deliver_response(shared, &id, &value, &raw),
            // An inbound request from the agent: an id AND a method. It is
            // blocked until we reply.
            (Some(id), Some(method)) => serve_request(shared, id, method, &value, &raw),
            // A notification: a method, no id. Never answered.
            (None, Some(method)) => take_notification(shared, method, &value, &raw),
            _ => true,
        };
        if !keep_reading {
            return;
        }
    }
}

/// Route a response: to the turn slot if it ends the live turn, otherwise to
/// the pending map. `false` means the reader should stop (channel gone or a
/// poisoned lock).
fn deliver_response(shared: &Shared, id: &Value, value: &Value, raw: &Arc<str>) -> bool {
    let Some(id_num) = id.as_i64() else {
        // We only ever allocate integer ids, so a non-integer id cannot be an
        // answer to anything of ours.
        return true;
    };

    let ends_turn = match shared.turn.lock() {
        Ok(mut slot) => {
            if *slot == Some(id_num) {
                *slot = None;
                true
            } else {
                false
            }
        }
        Err(_) => return false,
    };

    if ends_turn {
        // The outstanding table's lifetime is the *turn's*, not the bridge's:
        // an agent may legally end a turn (`cancelled`, `refusal`) without
        // waiting for a parked permission answer, and those ids die with it.
        drop_outstanding(shared, "the turn ended");
        if let Some(ev) = turn_spend(shared, value) {
            emit(&shared.tx, ev, None);
        }
        return emit(&shared.tx, turn_ended(value), Some(Arc::clone(raw)));
    }

    let sender = match shared.pending.lock() {
        Ok(mut guard) => guard.remove(&id_num),
        Err(_) => return false,
    };
    if let Some(sender) = sender {
        // If nobody is listening any more (the requester timed out and gave
        // up), silently drop — there is nothing left to do.
        let _ = sender.send(value.clone());
    }
    true
}

/// The `session/prompt` response's meaning.
///
/// A JSON-RPC `error` here **kills the turn, not the session**: the process
/// and the ACP session both stay alive and can be prompted again, so nothing
/// closes stdin and nothing marks the bridge dead. The failure is reported as
/// a [`StopReason::Failed`] turn and the conversation carries on.
fn turn_ended(response: &Value) -> AgentEvent {
    if let Some(error) = response.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        let detail = match error.get("data") {
            Some(data) if !data.is_null() => format!("{message}: {data}"),
            _ => message.to_string(),
        };
        return AgentEvent::TurnEnded {
            stop_reason: StopReason::Failed,
            error: Some(detail),
        };
    }
    let reason = response
        .get("result")
        .and_then(|result| result.get("stopReason"))
        .and_then(Value::as_str)
        .unwrap_or("end_turn");
    AgentEvent::TurnEnded {
        stop_reason: stop_reason_from(reason),
        error: None,
    }
}

/// The token breakdown a `session/prompt` response carries, as a
/// [`AgentEvent::UsageUpdate`] — the only place any ACP agent states one.
///
/// ACP v1 puts no token counts on `usage_update` (it reports occupancy and
/// cost, nothing else), but every adapter reports the turn's spend on the
/// prompt *response*: `result.usage` for `claude-code-acp` and `copilot`,
/// `result._meta.usage` for `grok`, which also flattens the same keys onto
/// `_meta` itself. Dropping it left three of four harnesses with no token
/// total at all.
///
/// **`totalTokens` is the authority, and `input` is derived from it.** The
/// adapters disagree on what `inputTokens` includes: `copilot` and `grok`
/// count cached reads inside it, `claude-code-acp` does not, so summing the
/// reported fields double-counts for two of them and [`Spend::total`] — which
/// is what the interface prints as the headline figure — would be wrong.
/// Subtracting the separately-reported parts from `totalTokens` instead gives
/// the fresh input tokens [`Spend::input`] is defined as, and makes
/// `Spend::total()` equal the agent's own `totalTokens` by construction, for
/// every adapter and without a per-harness branch.
///
/// `used`/`size` restate the last occupancy seen, and `cost` is `None`: the
/// cumulative-cost memo belongs to `usage_update`, and billing the same turn
/// twice is worse than not billing it here.
fn turn_spend(shared: &Shared, response: &Value) -> Option<AgentEvent> {
    let result = response.get("result")?;
    let meta = result.get("_meta");
    let usage = result
        .get("usage")
        .or_else(|| meta.and_then(|meta| meta.get("usage")))
        .or(meta)?;
    let count = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| usage.get(key).and_then(Value::as_u64))
            .unwrap_or(0)
    };
    let mut spend = Spend {
        input: count(&["inputTokens"]),
        output: count(&["outputTokens"]),
        thinking: count(&["thoughtTokens", "reasoningTokens"]),
        cache_read: count(&["cachedReadTokens"]),
        cache_creation: count(&["cachedWriteTokens", "cacheCreationTokens"]),
    };
    if let Some(total) = usage.get("totalTokens").and_then(Value::as_u64) {
        spend.input = total
            .saturating_sub(spend.output)
            .saturating_sub(spend.thinking)
            .saturating_sub(spend.cache_read)
            .saturating_sub(spend.cache_creation);
    }
    if spend.total() == 0 {
        return None;
    }
    let state = match shared.state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    let (used, size) = state.occupancy;
    Some(AgentEvent::UsageUpdate {
        used,
        size,
        cost: None,
        model: state.model.clone(),
        spend: Some(spend),
        origin: Origin::default(),
    })
}

/// ACP's five `StopReason` strings. An unknown one is [`StopReason::EndTurn`]:
/// the turn *is* over, and a new reason upstream grew is not a failure.
fn stop_reason_from(reason: &str) -> StopReason {
    match reason {
        "max_tokens" => StopReason::MaxTokens,
        "max_turn_requests" => StopReason::MaxTurnRequests,
        "refusal" => StopReason::Refusal,
        "cancelled" => StopReason::Cancelled,
        // "end_turn", and the fallback for anything unrecognised.
        _ => StopReason::EndTurn,
    }
}

/// Answer one inbound request from the agent. `false` means the reader should
/// stop.
///
/// Everything outside what `initialize` advertised is answered `-32601`; see
/// the module docs' "What we do not serve".
///
/// **INVARIANT: every inbound request gets exactly one reply, on every path,
/// success or failure.** The agent blocks on its request and has no timeout to
/// fall back on, so a path that returns without writing something is a
/// deadlock. `session/request_permission` is the hard case: its reply normally
/// comes much later, from whoever holds a sink, so every way of *failing to
/// park* the ask has to answer it `cancelled` here and now — `cancelled` is
/// precisely the protocol's "the client cannot answer this".
fn serve_request(shared: &Shared, id: Value, method: &str, value: &Value, raw: &Arc<str>) -> bool {
    let params = value.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "session/request_permission" => {
            // Record the raw id *before* emitting, so an answer that comes
            // back instantly still finds the entry it has to echo.
            let request_id = request_id_of(&id);
            let previous = match shared.outstanding.lock() {
                Ok(mut table) => table.insert(request_id.clone(), id.clone()),
                Err(_) => {
                    // No entry recorded, so no `AnswerPermission` could ever
                    // reach this ask — not even a hand-written one.
                    let _ =
                        write_line(shared, permission_reply(&id, &PermissionOutcome::Cancelled));
                    return true;
                }
            };
            if let Some(previous) = previous {
                // The agent reused a raw JSON-RPC id it had not yet been
                // answered on, which the table cannot represent twice. The
                // earlier ask is now unanswerable through it, so it is
                // answered here. Two asks sharing one id get two replies on
                // that id — the best available answer to a peer that broke
                // the id contract.
                tracing::debug!(
                    %request_id,
                    "acp agent reused an unanswered request id; cancelling the earlier ask"
                );
                let _ = write_line(
                    shared,
                    permission_reply(&previous, &PermissionOutcome::Cancelled),
                );
            }
            if emit(
                &shared.tx,
                permission_request(&request_id, &params),
                Some(Arc::clone(raw)),
            ) {
                return true;
            }
            // The event reached nobody, so nobody will ever answer it: retire
            // the entry and answer it ourselves. The reader still stops —
            // there is no consumer left for anything it reads.
            if let Ok(mut table) = shared.outstanding.lock() {
                table.remove(&request_id);
            }
            let _ = write_line(shared, permission_reply(&id, &PermissionOutcome::Cancelled));
            false
        }

        "fs/read_text_file" => reply(shared, &id, read_text_file(&params, &shared.root)),
        "fs/write_text_file" => reply(shared, &id, write_text_file(&params, &shared.root)),

        // NOTE: `terminal/*` is not an oversight. ACP's terminal is a one-shot
        // captured command with a byte cap on its output, not a pseudo-
        // terminal — so serving it would mean building a second, weaker
        // execution surface beside the real PTY this crate already owns. That
        // is a deliberate future decision, and until it is made, refusing a
        // capability we never advertised is the correct answer. The same goes
        // for `elicitation/create`, `mcp/*` and any `_`-prefixed vendor
        // method.
        _ => {
            tracing::debug!(
                method,
                "acp inbound request refused: capability not advertised"
            );
            if let Err(err) = write_line(shared, method_not_found(&id, method)) {
                // A failed enqueue must not stop the reader: keeping stdout
                // drained matters more than any single reply.
                tracing::debug!(%err, method, "acp refusal not written");
            }
            true
        }
    }
}

/// A JSON-RPC id rendered as the string key of the outstanding table. The id
/// may be a number or a string on the wire; a string key covers both without
/// losing which it was — the table stores the original `Value` to echo back.
///
/// The one-letter discriminator keeps the two id spaces apart: without it the
/// number `7` and the string `"7"` would render the same key and one ask would
/// evict the other. The key stays opaque to the consumer, which only ever
/// echoes it back through [`AgentInput::AnswerPermission`].
fn request_id_of(id: &Value) -> String {
    match id {
        Value::String(s) => format!("s:{s}"),
        Value::Number(n) => format!("n:{n}"),
        // Neither shape JSON-RPC allows for an id; rendered rather than
        // refused, so a nonconforming peer still gets an answerable ask.
        other => format!("j:{other}"),
    }
}

/// `session/request_permission`'s params as the event a consumer draws.
fn permission_request(request_id: &str, params: &Value) -> AgentEvent {
    AgentEvent::PermissionRequest {
        request_id: request_id.to_string(),
        tool_call: tool_call_update(params.get("toolCall")),
        options: permission_options(params.get("options")),
    }
}

/// The `toolCall` field is a `ToolCallUpdate` — the *patch* struct, so only
/// `toolCallId` is guaranteed. Mapped by synthesizing a `tool_call_update`
/// session update and running it through [`super::from_acp`], so the field
/// mapping lives in exactly one place.
fn tool_call_update(tool_call: Option<&Value>) -> ToolCallUpdate {
    let mut object = tool_call
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    object.insert("sessionUpdate".to_string(), json!("tool_call_update"));
    match super::from_acp(&Value::Object(object)) {
        Some(AgentEvent::ToolCallUpdate { update }) => update,
        _ => ToolCallUpdate::default(),
    }
}

/// The buttons the agent offered. ACP spells the id `optionId` (camelCase);
/// the model's field is `option_id`, and its serde is snake_case, so the
/// rename is done explicitly here rather than left to a `serde` round trip.
fn permission_options(options: Option<&Value>) -> Vec<PermissionOption> {
    options
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .map(|option| PermissionOption {
                    option_id: option
                        .get("optionId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    name: option
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    kind: permission_kind_from(
                        option
                            .get("kind")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    ),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// ACP's four `PermissionOptionKind` strings. `kind` is a display hint only —
/// `optionId` is what is acted on — so an unrecognised one falls back to the
/// most conservative reading rather than failing the frame.
fn permission_kind_from(kind: &str) -> PermissionKind {
    match kind {
        "allow_once" => PermissionKind::AllowOnce,
        "allow_always" => PermissionKind::AllowAlways,
        "reject_always" => PermissionKind::RejectAlways,
        // "reject_once", and the fallback for anything unrecognised.
        _ => PermissionKind::RejectOnce,
    }
}

/// The reply to a `session/request_permission`, echoing the agent's own id.
///
/// The word "outcome" appears **twice**, and that is correct: the response's
/// field is `outcome`, and the union inside it is internally tagged on
/// `outcome` too. The `optionId` echoed is always one the agent offered —
/// [`PermissionOutcome::Selected`] carries an id the consumer took from the
/// event, and nothing here invents one.
fn permission_reply(id: &Value, outcome: &PermissionOutcome) -> Value {
    let outcome = match outcome {
        PermissionOutcome::Selected { option_id } => {
            json!({"outcome": "selected", "optionId": option_id})
        }
        PermissionOutcome::Cancelled => json!({"outcome": "cancelled"}),
    };
    json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": outcome}})
}

/// Write the result-or-error of a request we do serve. Always `true`: a reply
/// that cannot be enqueued (stdin already closed) is logged, never a reason to
/// stop draining stdout.
fn reply(shared: &Shared, id: &Value, outcome: Result<Value, (i64, String)>) -> bool {
    let frame = match outcome {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => {
            tracing::debug!(code, %message, "acp inbound request failed");
            jsonrpc_error(id, code, &message)
        }
    };
    if let Err(err) = write_line(shared, frame) {
        tracing::debug!(%err, "acp reply not written");
    }
    true
}

/// `-32601`, the answer to any method we never advertised. The method name
/// goes in the message so a capture says which capability an agent wanted.
fn method_not_found(id: &Value, method: &str) -> Value {
    jsonrpc_error(id, -32601, &format!("method not found: {method}"))
}

fn jsonrpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// `fs/read_text_file` — `{sessionId, path, line?, limit?}` → `{content}`.
///
/// `path` must be absolute and inside the session root (`-32602` otherwise),
/// `line` is 1-based and `limit` is a maximum line count. A missing file is
/// `-32002` (ACP's "resource not found"); any other IO failure is `-32603`,
/// reported rather than dressed up.
fn read_text_file(params: &Value, root: &Path) -> Result<Value, (i64, String)> {
    let path = absolute_param(params, root)?;
    let text = std::fs::read_to_string(&path).map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => (-32002, format!("no such file: {}", path.display())),
        _ => (-32603, format!("could not read {}: {err}", path.display())),
    })?;
    let line = params.get("line").and_then(Value::as_u64);
    let limit = params.get("limit").and_then(Value::as_u64);
    Ok(json!({"content": slice_lines(&text, line, limit)}))
}

/// `fs/write_text_file` — `{sessionId, path, content}` → `{}`.
///
/// Parent directories are **not** created: the agent asked to write a file,
/// not to build a tree, and a missing parent is reported as the `-32002` it
/// is rather than silently papered over. The path must be inside the session
/// root; see [`absolute_param`].
fn write_text_file(params: &Value, root: &Path) -> Result<Value, (i64, String)> {
    let path = absolute_param(params, root)?;
    let content = params.get("content").and_then(Value::as_str).ok_or((
        -32602,
        "fs/write_text_file requires a `content` string".to_string(),
    ))?;
    std::fs::write(&path, content).map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => (
            -32002,
            format!("cannot write {}: no such path", path.display()),
        ),
        _ => (-32603, format!("could not write {}: {err}", path.display())),
    })?;
    Ok(json!({}))
}

/// The `path` param, which ACP requires to be absolute, resolved to the file
/// an `fs/*` request may actually touch.
///
/// A relative path is `-32602` (invalid params) rather than resolved: the
/// agent and this process need not share a working directory, so guessing
/// would read the wrong file.
///
/// The path is then **confined to the session root**. The host opts a session
/// into one directory tree, and `initialize` advertises `writeTextFile`
/// unconditionally, so without this check a prompt-injected or compromised
/// agent could write `~/.ssh/authorized_keys` or `~/.zshrc` through a
/// capability the host never granted per session. `..` is resolved
/// lexically *before* the prefix test — see [`lexically_normal`].
///
/// NOTE: symlink escapes are out of scope. A symlink inside the root that
/// points outside it still resolves outside when the file is opened, because
/// the check is deliberately textual (the target of a write need not exist
/// yet, so it cannot be `canonicalize`d). Closing that needs an
/// open-with-`O_NOFOLLOW`-per-component walk, which this crate has no unsafe
/// or libc surface for.
fn absolute_param(params: &Value, root: &Path) -> Result<PathBuf, (i64, String)> {
    let path = params
        .get("path")
        .and_then(Value::as_str)
        .ok_or((-32602, "a `path` string is required".to_string()))?;
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err((
            -32602,
            format!("`path` must be absolute: {}", path.display()),
        ));
    }
    let resolved = lexically_normal(&path);
    if !resolved.starts_with(root) {
        return Err((
            -32602,
            format!(
                "`path` is outside the session root {}: {}",
                root.display(),
                path.display()
            ),
        ));
    }
    Ok(resolved)
}

/// `line` (1-based start) and `limit` (maximum line count) applied to a file's
/// text. With neither, the text is returned verbatim.
///
/// A slice that reaches the **end** of the file keeps the file's own trailing
/// newline, so `line: 1, limit: <total lines>` returns exactly what an
/// unsliced read does. Without that, an agent that read a file both ways would
/// see a change nobody made.
fn slice_lines(text: &str, line: Option<u64>, limit: Option<u64>) -> String {
    if line.is_none() && limit.is_none() {
        return text.to_string();
    }
    let total = text.lines().count();
    let start = line.unwrap_or(1).saturating_sub(1) as usize;
    let available = total.saturating_sub(start);
    let taken = limit.map_or(available, |limit| (limit as usize).min(available));
    let picked: Vec<&str> = text.lines().skip(start).take(taken).collect();
    let mut out = picked.join("\n");
    if taken > 0 && start + taken >= total && text.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Route one notification. `session/update` is the only one that becomes an
/// event; `$/cancel_request`, `elicitation/complete` and anything unknown are
/// ignored, and **none of them is ever answered** — a notification has no id
/// to answer.
fn take_notification(shared: &Shared, method: &str, value: &Value, raw: &Arc<str>) -> bool {
    if method != "session/update" {
        return true;
    }
    let params = value.get("params").cloned().unwrap_or(Value::Null);
    let Some(ev) = session_update(shared, &params) else {
        // An unrecognised `sessionUpdate` is not an error — upstream's
        // vocabulary is open, and `from_acp` already says `None` for the ones
        // that are protocol-level rather than session updates.
        return true;
    };
    emit(&shared.tx, ev, Some(Arc::clone(raw)))
}

/// A `session/update` notification's params as an event, via
/// [`super::from_acp`] — the whole `sessionUpdate` vocabulary lives there and
/// is not reimplemented here.
///
/// The payload is `params["update"]`, with `params` itself as a defensive
/// fallback: the reference puts the update object under `update`, and an agent
/// that flattened it instead is still readable rather than silently dropped.
///
/// ACP allows `_meta` on the notification *and* on the update payload, and the
/// subagent markers Ubiq reads (see [`super::acp::meta_string`]) could be on
/// either, so the outer one is folded into the update — the update's own keys
/// win — before mapping. Then [`attribute`] applies what only the reader
/// knows: whose delegate a chunk belongs to, and what a cumulative cost
/// figure means as a delta.
fn session_update(shared: &Shared, params: &Value) -> Option<AgentEvent> {
    let mut update = params.get("update").unwrap_or(params).clone();
    if let Some(Value::Object(outer)) = params.get("_meta")
        && let Value::Object(inner) = &mut update
        && let Value::Object(meta) = inner.entry("_meta").or_insert_with(|| json!({}))
    {
        for (key, value) in outer {
            meta.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    if let Some(ev) = track_subagent(shared, &update) {
        return Some(ev);
    }
    let ev = super::from_acp(&update)?;
    Some(attribute(shared, params, &update, ev))
}

/// Record or forget a subagent session from the ACP subagent extension's two
/// `sessionUpdate` variants (draft PR #1992, advertised by
/// [`initialize_params`]). `None` means this notification was neither of them.
///
/// After a `subagent_spawned` the child's output arrives as ordinary
/// `session/update` notifications whose envelope `sessionId` is the
/// `subagentSessionId`, which is what the registry is for. The event returned
/// beside it is the *anchor*: the same shape `io/jsonl.rs` gives a Claude
/// `Task` block — a `ToolKind::Delegate` call, in progress, titled with the
/// spawn's description — so the extension path and the heuristic fallback
/// converge on one transcript shape and the chat panel needs no second
/// rendering path. Its id is the `subagentSessionId`, which is what the
/// children's `Origin::parent_tool_use_id` then points at. Every state the
/// draft defines is terminal, so a `subagent_state_update` closes the call.
///
/// The anchor is deliberately not routed through [`attribute`]: a spawn
/// belongs to whoever opened it, never to itself.
fn track_subagent(shared: &Shared, update: &Value) -> Option<AgentEvent> {
    let id = update.get("subagentSessionId").and_then(Value::as_str)?;
    let kind = update.get("sessionUpdate").and_then(Value::as_str)?;
    let mut state = match shared.state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    match kind {
        "subagent_spawned" => {
            let name = update.get("name").and_then(Value::as_str).unwrap_or(id);
            state.subagents.insert(id.to_string(), name.to_string());
            let mut call = ToolCall::new(id, name);
            call.kind = ToolKind::Delegate;
            call.status = ToolStatus::InProgress;
            // The two field names `io/jsonl.rs` reads off a `Task` input, and
            // the two `attribute` reads back off `raw_input` — this is where
            // the spawn's `task` string lives.
            call.raw_input = Some(json!({
                "description": update.get("task").cloned().unwrap_or(Value::Null),
                "subagent_type": name,
            }));
            Some(AgentEvent::ToolCall { call })
        }
        "subagent_state_update" => {
            state.subagents.remove(id);
            state.cost_totals.remove(id);
            // `ToolStatus` has no cancelled variant, so every non-completion
            // is a failure — the draft's `failed`, `cancelled` and
            // `disconnected` all end the call without a result.
            let status = match update.get("state").and_then(Value::as_str) {
                Some("completed") => ToolStatus::Completed,
                _ => ToolStatus::Failed,
            };
            Some(AgentEvent::ToolCallUpdate {
                update: ToolCallUpdate::finished(id, status),
            })
        }
        _ => None,
    }
}

/// Apply the reader's own knowledge to one freshly mapped event: the usage
/// contract, and delegate attribution.
///
/// **Usage.** ACP's `cost.amount` is session-cumulative while
/// [`AgentEvent::UsageUpdate`]'s `cost` is contractually a per-report delta
/// (rule 5), so the running total is subtracted here — the same shape
/// [`super::jsonl`] uses for Claude's cumulative figure. `spend` stays `None`:
/// ACP reports no token breakdown at all, and synthesising one would break the
/// same contract in the other direction.
///
/// **Delegates.** An adapter that took [`initialize_params`]'s subagent
/// capability says so exactly: the child's output arrives on the child's own
/// `sessionId`, which [`track_subagent`] has already mapped to a name. Failing
/// that, ACP v1 has no subagent vocabulary, so an agent that writes no
/// `_meta` marker leaves only the shape of the traffic to go on: while a
/// `Task`/`Agent` tool call is open the main agent is blocked on it, so every
/// unattributed chunk that arrives is the delegate's. An origin `_meta` already
/// filled is never overwritten.
///
/// ponytail: with two delegates open at once every chunk is attributed to the
/// most recently opened one — nothing on the wire could tell them apart. The
/// upgrade path is the `_meta` marker, which an agent can send today.
fn attribute(shared: &Shared, params: &Value, update: &Value, mut ev: AgentEvent) -> AgentEvent {
    let mut state = match shared.state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    let session = params
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let child = state
        .subagents
        .get(&session)
        .map(|name| (session.clone(), Some(name.clone())));
    match &mut ev {
        AgentEvent::UsageUpdate { cost, model, .. } => {
            if let Some(reported) = cost {
                let total = reported.amount;
                let seen = state.cost_totals.entry(session).or_default();
                reported.amount = (total - *seen).max(0.0);
                *seen = total;
                if reported.amount <= 0.0 {
                    *cost = None;
                }
            }
            model.clone_from(&state.model);
            if let AgentEvent::UsageUpdate { used, size, .. } = &ev {
                state.occupancy = (*used, *size);
            }
        }
        AgentEvent::ConfigOptionUpdate { .. } => {
            if let Some(model) = update
                .get("configOptions")
                .and_then(|options| config_current_value(options, "model"))
            {
                state.model = Some(model);
            }
            return ev;
        }
        AgentEvent::ToolCall { call } if is_delegate(update, &call.title) => {
            call.kind = ToolKind::Delegate;
            let subagent = call
                .raw_input
                .as_ref()
                .and_then(|raw| str_any(raw, &["subagent_type", "subagent", "description"]));
            state.delegates.push((call.id.clone(), subagent));
            // The delegate's own call belongs to whoever opened it, not to itself.
            return ev;
        }
        AgentEvent::ToolCallUpdate { update } => {
            if matches!(
                update.status,
                Some(ToolStatus::Completed | ToolStatus::Failed)
            ) {
                state.delegates.retain(|(id, _)| id != &update.id);
            }
            return ev;
        }
        _ => {}
    }
    // Precedence: an explicit `_meta` origin (checked below), then the child
    // session id, then the open-delegate heuristic.
    let Some((id, subagent)) = child.or_else(|| state.delegates.last().cloned()) else {
        return ev;
    };
    let origin = match &mut ev {
        AgentEvent::AgentMessageChunk { origin, .. }
        | AgentEvent::AgentThoughtChunk { origin, .. }
        | AgentEvent::UsageUpdate { origin, .. } => origin,
        AgentEvent::ToolCall { call } => &mut call.origin,
        _ => return ev,
    };
    if origin.parent_tool_use_id.is_none() {
        origin.parent_tool_use_id = Some(id);
        origin.subagent_type = subagent;
    }
    ev
}

/// Whether a `tool_call` is a delegation — the spawn of a subagent.
///
/// ACP's `ToolKind` has no value for it, so the name is all there is: the
/// `_meta` tool name the reference adapter emits
/// (`_meta.claudeCode.toolName`), or the call's own title or name.
fn is_delegate(update: &Value, title: &str) -> bool {
    let named =
        |name: &str| name.eq_ignore_ascii_case("task") || name.eq_ignore_ascii_case("agent");
    super::acp::meta_string(update, &["toolName"]).is_some_and(|name| named(&name))
        || named(title)
        || str_any(update, &["name"]).is_some_and(|name| named(&name))
}

/// The first of `keys` present on `value` as a string.
fn str_any(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
        .map(String::from)
}

/// Push one mapped event, logged at `debug`. `false` means nobody is
/// listening any more and the reader should stop.
fn emit(tx: &mpsc::Sender<Option<Framed>>, ev: AgentEvent, raw: Option<Arc<str>>) -> bool {
    tracing::debug!(event = ?ev, "acp event");
    tx.send(Some((ev, raw))).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{Content, ToolStatus};

    fn parse(json: &str) -> Value {
        serde_json::from_str(json).unwrap()
    }

    /// A fresh directory to be a session root, unique per test.
    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("am-acp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        lexically_normal(&root)
    }

    /// A [`Shared`] with no child process behind it. The two receivers are the
    /// writer thread's queue and the event channel, so a test can read exactly
    /// what the bridge enqueued and emitted — and, because nothing drains the
    /// queue, prove that a write never blocks on one.
    #[allow(clippy::type_complexity)]
    fn test_shared(
        root: &Path,
    ) -> (
        Arc<Shared>,
        mpsc::Receiver<Option<Value>>,
        mpsc::Receiver<Option<Framed>>,
    ) {
        let (tx, events) = mpsc::channel();
        let (writes, written) = mpsc::channel();
        let shared = Arc::new(Shared {
            writes,
            pending: Arc::new(Mutex::new(HashMap::new())),
            outstanding: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicI64::new(1)),
            session_id: Arc::new(OnceLock::new()),
            turn: Arc::new(Mutex::new(None)),
            prompt_caps: Arc::new(OnceLock::new()),
            root: root.to_path_buf(),
            tx,
            state: Mutex::new(ReaderState::default()),
        });
        (shared, written, events)
    }

    /// Every frame enqueued for the writer thread so far, in order.
    fn frames(written: &mpsc::Receiver<Option<Value>>) -> Vec<Value> {
        let mut out = Vec::new();
        while let Ok(Some(value)) = written.try_recv() {
            out.push(value);
        }
        out
    }

    fn raw_of(value: &Value) -> Arc<str> {
        Arc::from(value.to_string().as_str())
    }

    // ── the turn ───────────────────────────────────────────────────────

    #[test]
    fn every_acp_stop_reason_maps_and_an_unknown_one_ends_the_turn() {
        assert_eq!(stop_reason_from("end_turn"), StopReason::EndTurn);
        assert_eq!(stop_reason_from("max_tokens"), StopReason::MaxTokens);
        assert_eq!(
            stop_reason_from("max_turn_requests"),
            StopReason::MaxTurnRequests
        );
        assert_eq!(stop_reason_from("refusal"), StopReason::Refusal);
        assert_eq!(stop_reason_from("cancelled"), StopReason::Cancelled);
        assert_eq!(stop_reason_from("something_new"), StopReason::EndTurn);
    }

    #[test]
    fn a_prompt_response_ends_the_turn_with_its_stop_reason() {
        let response = parse(r#"{"jsonrpc":"2.0","id":7,"result":{"stopReason":"cancelled"}}"#);
        assert_eq!(
            turn_ended(&response),
            AgentEvent::TurnEnded {
                stop_reason: StopReason::Cancelled,
                error: None,
            }
        );
    }

    /// A JSON-RPC error on `session/prompt` fails the turn and nothing more —
    /// the session survives it, which is why this is an event rather than a
    /// returned `Err`.
    #[test]
    fn an_error_response_fails_the_turn_and_carries_its_message() {
        let response = parse(
            r#"{"jsonrpc":"2.0","id":7,
                "error":{"code":-32603,"message":"model unavailable","data":{"retry":false}}}"#,
        );
        let AgentEvent::TurnEnded { stop_reason, error } = turn_ended(&response) else {
            panic!("expected a turn to end");
        };
        assert_eq!(stop_reason, StopReason::Failed);
        let error = error.expect("a failed turn explains itself");
        assert!(error.contains("model unavailable"), "error was: {error}");
        assert!(error.contains("retry"), "error was: {error}");
    }

    // ── permissions ────────────────────────────────────────────────────

    #[test]
    fn a_permission_request_renames_option_id_and_maps_all_four_kinds() {
        let params = parse(
            r#"{"sessionId":"s1",
                "toolCall":{"toolCallId":"call_1","title":"Run a command","status":"pending"},
                "options":[
                  {"optionId":"a1","name":"Allow once","kind":"allow_once"},
                  {"optionId":"a2","name":"Always allow","kind":"allow_always"},
                  {"optionId":"r1","name":"Reject once","kind":"reject_once"},
                  {"optionId":"r2","name":"Always reject","kind":"reject_always"}]}"#,
        );
        let AgentEvent::PermissionRequest {
            request_id,
            tool_call,
            options,
        } = permission_request("3", &params)
        else {
            panic!("expected a permission request");
        };
        assert_eq!(request_id, "3");
        assert_eq!(tool_call.id, "call_1");
        assert_eq!(tool_call.title.as_deref(), Some("Run a command"));
        assert_eq!(tool_call.status, Some(ToolStatus::Pending));
        let ids: Vec<&str> = options.iter().map(|o| o.option_id.as_str()).collect();
        assert_eq!(ids, vec!["a1", "a2", "r1", "r2"]);
        let kinds: Vec<PermissionKind> = options.iter().map(|o| o.kind).collect();
        assert_eq!(
            kinds,
            vec![
                PermissionKind::AllowOnce,
                PermissionKind::AllowAlways,
                PermissionKind::RejectOnce,
                PermissionKind::RejectAlways,
            ]
        );
    }

    /// Only `toolCallId` is guaranteed inside `toolCall` — it is the patch
    /// struct — so a bare one still produces a usable event.
    #[test]
    fn a_permission_request_survives_a_tool_call_with_only_an_id() {
        let params = parse(r#"{"toolCall":{"toolCallId":"call_2"},"options":[]}"#);
        let AgentEvent::PermissionRequest { tool_call, .. } = permission_request("x", &params)
        else {
            panic!("expected a permission request");
        };
        assert_eq!(tool_call.id, "call_2");
        assert_eq!(tool_call.title, None);
    }

    /// The word "outcome" appears twice, and the agent's own id — number or
    /// string — comes back unchanged.
    #[test]
    fn the_permission_reply_nests_outcome_twice_and_echoes_the_id() {
        let selected = permission_reply(
            &json!(3),
            &PermissionOutcome::Selected {
                option_id: "a1".to_string(),
            },
        );
        assert_eq!(
            selected,
            json!({"jsonrpc":"2.0","id":3,
                   "result":{"outcome":{"outcome":"selected","optionId":"a1"}}})
        );

        let cancelled = permission_reply(&json!("req-9"), &PermissionOutcome::Cancelled);
        assert_eq!(
            cancelled,
            json!({"jsonrpc":"2.0","id":"req-9",
                   "result":{"outcome":{"outcome":"cancelled"}}})
        );
    }

    // ── requests we refuse ─────────────────────────────────────────────

    #[test]
    fn an_unadvertised_request_is_answered_method_not_found() {
        assert_eq!(
            method_not_found(&json!(11), "terminal/create"),
            json!({"jsonrpc":"2.0","id":11,
                   "error":{"code":-32601,"message":"method not found: terminal/create"}})
        );
    }

    // ── the filesystem we do serve ─────────────────────────────────────

    #[test]
    fn read_text_file_honours_line_and_limit() {
        let dir = temp_root("read");
        let path = dir.join("lines.txt");
        std::fs::write(&path, "one\ntwo\nthree\nfour\n").unwrap();

        let params = json!({"sessionId":"s1","path": path.display().to_string()});
        assert_eq!(
            read_text_file(&params, &dir).unwrap(),
            json!({"content":"one\ntwo\nthree\nfour\n"})
        );

        let sliced = json!({"sessionId":"s1","path": path.display().to_string(),
                            "line": 2, "limit": 2});
        assert_eq!(
            read_text_file(&sliced, &dir).unwrap(),
            json!({"content":"two\nthree"})
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_text_file_reports_a_missing_path_as_resource_not_found() {
        let dir = temp_root("missing");
        let missing = dir.join("am-acp-definitely-not-here.txt");
        let params = json!({"sessionId":"s1","path": missing.display().to_string()});
        let (code, message) = read_text_file(&params, &dir).unwrap_err();
        assert_eq!(code, -32002);
        assert!(message.contains("no such file"), "message was: {message}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_relative_path_is_invalid_params() {
        let root = Path::new("/session/root");
        let (code, _) = read_text_file(&json!({"path":"relative/file.txt"}), root).unwrap_err();
        assert_eq!(code, -32602);
        let (code, _) =
            write_text_file(&json!({"path":"relative/file.txt","content":""}), root).unwrap_err();
        assert_eq!(code, -32602);
    }

    /// Fix 9. `writeTextFile` is advertised unconditionally, so the session
    /// root is the only thing between a compromised agent and `~/.zshrc`.
    #[test]
    fn an_fs_path_outside_the_session_root_is_refused() {
        let root = temp_root("confine");

        // A `..` escape, resolved lexically before the prefix test.
        let escape = root.join("..").join("am-acp-escaped.txt");
        let (code, message) = write_text_file(
            &json!({"path": escape.display().to_string(), "content":"pwned"}),
            &root,
        )
        .unwrap_err();
        assert_eq!(code, -32602);
        assert!(
            message.contains("outside the session root"),
            "message was: {message}"
        );
        assert!(
            !lexically_normal(&escape).exists(),
            "the refused write must not have happened"
        );

        // An unrelated absolute path, for read and for write alike.
        let (code, _) = read_text_file(&json!({"path":"/etc/passwd"}), &root).unwrap_err();
        assert_eq!(code, -32602);
        let unrelated = std::env::temp_dir().join("am-acp-unrelated.txt");
        let (code, _) = write_text_file(
            &json!({"path": unrelated.display().to_string(), "content":"pwned"}),
            &root,
        )
        .unwrap_err();
        assert_eq!(code, -32602);
        assert!(!unrelated.exists());

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The other half of Fix 9: confinement must not break the legitimate
    /// case, including a nested directory and a `.` in the path.
    #[test]
    fn a_nested_path_inside_the_session_root_is_served() {
        let root = temp_root("nested");
        std::fs::create_dir_all(root.join("src/inner")).unwrap();
        let path = root.join("src/./inner/ok.txt");

        write_text_file(
            &json!({"path": path.display().to_string(), "content":"fine\n"}),
            &root,
        )
        .unwrap();
        assert_eq!(
            read_text_file(&json!({"path": path.display().to_string()}), &root).unwrap(),
            json!({"content":"fine\n"})
        );
        assert_eq!(
            std::fs::read_to_string(root.join("src/inner/ok.txt")).unwrap(),
            "fine\n"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The trailing-newline fix: `line: 1, limit: <total>` has to return what
    /// an unsliced read returns, or an agent reading a file both ways sees a
    /// change nobody made.
    #[test]
    fn a_slice_that_reaches_the_end_of_the_file_keeps_its_trailing_newline() {
        let text = "one\ntwo\nthree\n";
        assert_eq!(slice_lines(text, Some(1), Some(3)), text);
        assert_eq!(slice_lines(text, Some(1), Some(9)), text);
        assert_eq!(slice_lines(text, Some(2), None), "two\nthree\n");
        assert_eq!(slice_lines(text, None, None), text);
        // A slice that stops short must NOT invent one.
        assert_eq!(slice_lines(text, Some(1), Some(2)), "one\ntwo");
        // A file with no trailing newline never grows one.
        assert_eq!(slice_lines("one\ntwo", Some(1), Some(2)), "one\ntwo");
        // Degenerate cases: a bare newline, and a start past the end.
        assert_eq!(slice_lines("\n", Some(1), Some(1)), "\n");
        assert_eq!(slice_lines(text, Some(9), Some(2)), "");
    }

    // ── notifications ──────────────────────────────────────────────────

    #[test]
    fn a_session_update_becomes_the_event_from_acp_maps_it_to() {
        let (shared, _written, _events) = test_shared(&temp_root("update"));
        let params = parse(
            r#"{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk",
                "content":{"type":"text","text":"hello"},"messageId":"m1"}}"#,
        );
        assert_eq!(
            session_update(&shared, &params),
            Some(AgentEvent::AgentMessageChunk {
                content: Content::text("hello"),
                message_id: Some("m1".to_string()),
                origin: Default::default(),
            })
        );
    }

    #[test]
    fn an_unknown_session_update_produces_no_event() {
        let (shared, _written, _events) = test_shared(&temp_root("unknown-update"));
        let params = parse(r#"{"sessionId":"s1","update":{"sessionUpdate":"something_new"}}"#);
        assert_eq!(session_update(&shared, &params), None);
    }

    /// A marker on the *notification* reaches [`super::from_acp`] too: ACP
    /// allows `_meta` at either level, so the outer one is folded in.
    #[test]
    fn a_notification_level_meta_marker_attributes_a_chunk() {
        let (shared, _written, _events) = test_shared(&temp_root("outer-meta"));
        let params = parse(
            r#"{"sessionId":"s1","_meta":{"claudeCode":{"parentToolUseId":"t9",
                "subagentType":"explorer"}},
                "update":{"sessionUpdate":"agent_message_chunk",
                "content":{"type":"text","text":"hi"}}}"#,
        );
        let Some(AgentEvent::AgentMessageChunk { origin, .. }) = session_update(&shared, &params)
        else {
            panic!("expected a chunk");
        };
        assert_eq!(origin.parent_tool_use_id.as_deref(), Some("t9"));
        assert_eq!(origin.subagent_type.as_deref(), Some("explorer"));
    }

    /// ── delegate inference ──
    ///
    /// A `Task` tool call opens a delegate, everything until its completion is
    /// attributed to it, and everything after is the main agent's again.
    #[test]
    fn a_chunk_inside_an_open_task_call_is_attributed_to_it() {
        let (shared, _written, _events) = test_shared(&temp_root("delegate"));
        let chunk = parse(
            r#"{"update":{"sessionUpdate":"agent_message_chunk",
                "content":{"type":"text","text":"hi"}}}"#,
        );

        // Before the delegate opens: nobody's but the conversation's.
        let Some(AgentEvent::AgentMessageChunk { origin, .. }) = session_update(&shared, &chunk)
        else {
            panic!("expected a chunk");
        };
        assert_eq!(origin, Default::default());

        // The `Task` call itself, recognised by the reference adapter's
        // `_meta.claudeCode.toolName`, and promoted to a delegation kind.
        let call = parse(
            r#"{"update":{"sessionUpdate":"tool_call","toolCallId":"t1","title":"Explore",
                "kind":"other","status":"in_progress",
                "rawInput":{"subagent_type":"explorer"},
                "_meta":{"claudeCode":{"toolName":"Task"}}}}"#,
        );
        let Some(AgentEvent::ToolCall { call }) = session_update(&shared, &call) else {
            panic!("expected a tool call");
        };
        assert_eq!(call.kind, ToolKind::Delegate);
        // The spawn belongs to whoever opened it, not to itself.
        assert_eq!(call.origin, Default::default());

        let Some(AgentEvent::AgentMessageChunk { origin, .. }) = session_update(&shared, &chunk)
        else {
            panic!("expected a chunk");
        };
        assert_eq!(origin.parent_tool_use_id.as_deref(), Some("t1"));
        assert_eq!(origin.subagent_type.as_deref(), Some("explorer"));

        let done = parse(
            r#"{"update":{"sessionUpdate":"tool_call_update","toolCallId":"t1",
                "status":"completed"}}"#,
        );
        session_update(&shared, &done).unwrap();

        let Some(AgentEvent::AgentMessageChunk { origin, .. }) = session_update(&shared, &chunk)
        else {
            panic!("expected a chunk");
        };
        assert_eq!(origin, Default::default());
    }

    /// ACP states `cost.amount` cumulatively; the event contracts for a
    /// per-report delta, and a report that adds nothing carries no cost.
    #[test]
    fn a_cumulative_acp_cost_becomes_a_per_report_delta() {
        let (shared, _written, _events) = test_shared(&temp_root("cost"));
        let usage = |amount: &str| {
            parse(&format!(
                r#"{{"update":{{"sessionUpdate":"usage_update","used":10,"size":100,
                    "cost":{{"amount":{amount},"currency":"USD"}}}}}}"#
            ))
        };
        let cost = |params: &Value| match session_update(&shared, params) {
            Some(AgentEvent::UsageUpdate { cost, .. }) => cost,
            other => panic!("expected usage, got {other:?}"),
        };
        assert_eq!(cost(&usage("0.05")).map(|c| c.amount), Some(0.05));
        // 0.18 cumulative is 0.13 more than the 0.05 already reported.
        let delta = cost(&usage("0.18")).unwrap();
        assert!((delta.amount - 0.13).abs() < 1e-9, "{}", delta.amount);
        // Nothing new to bill: no cost at all rather than a zero.
        assert_eq!(cost(&usage("0.18")), None);
    }

    /// After a `subagent_spawned`, the child's output arrives on the child's
    /// own envelope `sessionId` — exact attribution, no heuristic involved,
    /// and it stops at the terminal `subagent_state_update`.
    #[test]
    fn a_child_session_attributes_its_chunks_to_the_subagent() {
        let (shared, _written, _events) = test_shared(&temp_root("child"));
        let spawned = parse(
            r#"{"sessionId":"parent","update":{"sessionUpdate":"subagent_spawned",
                "subagentSessionId":"child_1","name":"Explore","task":"look around",
                "capabilities":{}}}"#,
        );
        assert!(session_update(&shared, &spawned).is_some());

        let chunk = parse(
            r#"{"sessionId":"child_1","update":{"sessionUpdate":"agent_message_chunk",
                "content":{"type":"text","text":"hi"}}}"#,
        );
        let origin = |params: &Value| match session_update(&shared, params) {
            Some(AgentEvent::AgentMessageChunk { origin, .. }) => origin,
            other => panic!("expected a chunk, got {other:?}"),
        };
        let attributed = origin(&chunk);
        assert_eq!(attributed.parent_tool_use_id.as_deref(), Some("child_1"));
        assert_eq!(attributed.subagent_type.as_deref(), Some("Explore"));

        let done = parse(
            r#"{"sessionId":"parent","update":{"sessionUpdate":"subagent_state_update",
                "subagentSessionId":"child_1","state":"completed"}}"#,
        );
        assert!(session_update(&shared, &done).is_some());
        assert_eq!(origin(&chunk), Default::default());
    }

    /// Every ACP agent states the turn's tokens on the `session/prompt`
    /// response and nowhere else, in three different places and two different
    /// conventions for what `inputTokens` includes. All three are pinned from
    /// real captures, and `Spend::total()` — the interface's headline figure —
    /// must equal the agent's own `totalTokens` in every one.
    #[test]
    fn a_prompt_response_reports_the_turns_tokens() {
        let (shared, _written, _events) = test_shared(&temp_root("spend"));
        let spend = |response: &str| match turn_spend(&shared, &parse(response)) {
            Some(AgentEvent::UsageUpdate {
                used, size, spend, ..
            }) => (used, size, spend.unwrap()),
            other => panic!("expected usage, got {other:?}"),
        };

        // `claude-code-acp`: `result.usage`, and `inputTokens` excludes the
        // cache — 3 + 17 + 19170 is its own 19190.
        let (_, _, claude) = spend(
            r#"{"result":{"stopReason":"end_turn","usage":{"inputTokens":3,"outputTokens":17,
                "cachedReadTokens":0,"cachedWriteTokens":19170,"totalTokens":19190}}}"#,
        );
        assert_eq!(claude.total(), 19_190);
        assert_eq!(claude.input, 3);
        assert_eq!(claude.cache_creation, 19_170);

        // `copilot`: same place, but `inputTokens` *includes* the 30080
        // cached reads, so taken verbatim it would sum to 76702 rather than
        // 46622. The fresh input is the 16270 that is left.
        let (_, _, copilot) = spend(
            r#"{"result":{"stopReason":"end_turn","usage":{"inputTokens":46350,
                "outputTokens":272,"totalTokens":46622,"thoughtTokens":0,
                "cachedReadTokens":30080,"cachedWriteTokens":0}}}"#,
        );
        assert_eq!(copilot.total(), 46_622);
        assert_eq!(copilot.input, 16_270);
        assert_eq!(copilot.cache_read, 30_080);

        // `grok`: under `result._meta.usage`, with reasoning counted apart.
        let (used, size, grok) = spend(
            r#"{"result":{"stopReason":"end_turn","_meta":{"modelId":"grok-4.6",
                "usage":{"inputTokens":16083,"outputTokens":54,"totalTokens":16137,
                "cachedReadTokens":11776,"cacheCreationTokens":0,"reasoningTokens":36}}}}"#,
        );
        assert_eq!(grok.total(), 16_137);
        assert_eq!(grok.input, 4_271);
        assert_eq!(grok.thinking, 36);
        // Grok sends no `usage_update` at all, so it names no window — 0 is
        // the honest answer rather than an invented one.
        assert_eq!((used, size), (0, 0));

        // A response with no usage anywhere reports nothing.
        assert!(turn_spend(&shared, &parse(r#"{"result":{"stopReason":"end_turn"}}"#)).is_none());
    }

    /// A turn's spend restates the occupancy the last `usage_update` gave, so
    /// the context ring does not read as a window that just emptied.
    #[test]
    fn a_turns_spend_restates_the_last_known_occupancy() {
        let (shared, _written, _events) = test_shared(&temp_root("occupancy"));
        session_update(
            &shared,
            &parse(
                r#"{"sessionId":"s","update":{"sessionUpdate":"usage_update",
                    "used":19175,"size":1000000}}"#,
            ),
        );
        let Some(AgentEvent::UsageUpdate { used, size, .. }) = turn_spend(
            &shared,
            &parse(r#"{"result":{"usage":{"inputTokens":3,"outputTokens":17,"totalTokens":20}}}"#),
        ) else {
            panic!("expected usage");
        };
        assert_eq!((used, size), (19_175, 1_000_000));
    }

    /// A spawn gets the same anchor `io/jsonl.rs` gives a Claude `Task` block —
    /// a delegation call the transcript can hang the subagent's tab off — and
    /// it belongs to whoever opened it, not to itself.
    #[test]
    fn a_spawn_synthesises_the_delegate_call_the_transcript_anchors_on() {
        let (shared, _written, _events) = test_shared(&temp_root("anchor"));
        let spawned = parse(
            r#"{"sessionId":"parent","update":{"sessionUpdate":"subagent_spawned",
                "subagentSessionId":"child_1","name":"Explore","task":"look around",
                "capabilities":{}}}"#,
        );
        let Some(AgentEvent::ToolCall { call }) = session_update(&shared, &spawned) else {
            panic!("expected the anchor tool call");
        };
        // The id the children's `parent_tool_use_id` will point at.
        assert_eq!(call.id, "child_1");
        assert_eq!(call.title, "Explore");
        assert_eq!(call.kind, ToolKind::Delegate);
        assert_eq!(call.status, ToolStatus::InProgress);
        assert_eq!(
            call.raw_input.as_ref().unwrap()["description"],
            "look around"
        );
        // The spawn is the parent's line, so it carries no origin of its own.
        assert_eq!(call.origin, Default::default());

        // `cancelled` is not a `ToolStatus`, so a non-completion is a failure.
        for (state, want) in [
            ("completed", ToolStatus::Completed),
            ("cancelled", ToolStatus::Failed),
        ] {
            let done = parse(&format!(
                r#"{{"sessionId":"parent","update":{{"sessionUpdate":"subagent_state_update",
                    "subagentSessionId":"child_1","state":"{state}"}}}}"#
            ));
            let Some(AgentEvent::ToolCallUpdate { update }) = session_update(&shared, &done) else {
                panic!("expected the anchor to close");
            };
            assert_eq!(update.id, "child_1");
            assert_eq!(update.status, Some(want));
        }
    }

    /// Precedence: an agent that states an origin in `_meta` outranks the
    /// child session id the envelope would otherwise supply.
    #[test]
    fn an_explicit_meta_origin_beats_the_child_session_id() {
        let (shared, _written, _events) = test_shared(&temp_root("child_meta"));
        let spawned = parse(
            r#"{"sessionId":"parent","update":{"sessionUpdate":"subagent_spawned",
                "subagentSessionId":"child_1","name":"Explore","task":"look",
                "capabilities":{}}}"#,
        );
        session_update(&shared, &spawned);
        let chunk = parse(
            r#"{"sessionId":"child_1","update":{"sessionUpdate":"agent_message_chunk",
                "content":{"type":"text","text":"hi"},
                "_meta":{"parentToolCallId":"t7","subagentType":"stated"}}}"#,
        );
        let Some(AgentEvent::AgentMessageChunk { origin, .. }) = session_update(&shared, &chunk)
        else {
            panic!("expected a chunk");
        };
        assert_eq!(origin.parent_tool_use_id.as_deref(), Some("t7"));
        assert_eq!(origin.subagent_type.as_deref(), Some("stated"));
    }

    /// A subagent reports its own context window and its own spend, so its
    /// cumulative figure is memoed under its own session and cannot move the
    /// parent's delta.
    #[test]
    fn a_subagent_cost_does_not_disturb_the_parent_delta() {
        let (shared, _written, _events) = test_shared(&temp_root("child_cost"));
        let usage = |session: &str, amount: &str| {
            parse(&format!(
                r#"{{"sessionId":"{session}","update":{{"sessionUpdate":"usage_update",
                    "used":10,"size":100,
                    "cost":{{"amount":{amount},"currency":"USD"}}}}}}"#
            ))
        };
        let cost = |params: &Value| match session_update(&shared, params) {
            Some(AgentEvent::UsageUpdate { cost, .. }) => cost,
            other => panic!("expected usage, got {other:?}"),
        };
        session_update(
            &shared,
            &parse(
                r#"{"sessionId":"parent","update":{"sessionUpdate":"subagent_spawned",
                    "subagentSessionId":"child_1","name":"Explore","task":"look",
                    "capabilities":{}}}"#,
            ),
        );
        assert_eq!(cost(&usage("parent", "0.05")).map(|c| c.amount), Some(0.05));
        // The child's 2.00 is its own running total, not the parent's.
        assert_eq!(cost(&usage("child_1", "2.00")).map(|c| c.amount), Some(2.0));
        let delta = cost(&usage("parent", "0.18")).unwrap();
        assert!((delta.amount - 0.13).abs() < 1e-9, "{}", delta.amount);
    }

    // ── the frames we write ────────────────────────────────────────────

    /// Pinned verbatim: `protocolVersion` is the integer 1, the two `fs`
    /// flags and `session.configOptions.boolean` are advertised, and
    /// `terminal` and `elicitation` are absent.
    #[test]
    fn the_initialize_params_are_exactly_what_we_advertise() {
        assert_eq!(
            initialize_params(),
            json!({
                "protocolVersion": 1,
                "clientInfo": {
                    "name": "agent-manager",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "clientCapabilities": {
                    "fs": {"readTextFile": true, "writeTextFile": true},
                    "session": {"configOptions": {"boolean": {}}},
                    "_meta": {"jetbrains": {"air": {
                        "version": 1,
                        "capabilities": ["nativeSubagentSessions"],
                    }}},
                },
            })
        );
    }

    /// The subagent extension is gated on this exact `_meta` path; the
    /// adapters read nothing else, and drop back to a plain tool call without
    /// it. `asyncTasks` and `sessionFailure` are deliberately absent — we
    /// handle neither.
    #[test]
    fn initialize_advertises_the_native_subagent_capability() {
        let air = &initialize_params()["clientCapabilities"]["_meta"]["jetbrains"]["air"];
        assert_eq!(air["version"], 1);
        assert_eq!(air["capabilities"], json!(["nativeSubagentSessions"]));
    }

    /// A cancel is a notification: no `id` key at all, so nothing waits for a
    /// response that never comes.
    #[test]
    fn session_cancel_is_written_as_a_notification() {
        let frame = cancel_notification("sess_1");
        assert_eq!(
            frame,
            json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"sess_1"}})
        );
        assert!(frame.get("id").is_none(), "frame was: {frame}");
    }

    /// `type: "boolean"` for a boolean, and no `type` at all for a string —
    /// the string form is the default variant and must not carry one.
    #[test]
    fn set_config_option_types_only_the_boolean_value() {
        assert_eq!(
            set_config_params("s1", "model", &ConfigSetting::Text("opus".to_string())),
            json!({"sessionId":"s1","configId":"model","value":"opus"})
        );
        let flag = set_config_params("s1", "thinking", &ConfigSetting::Flag(true));
        assert_eq!(
            flag,
            json!({"sessionId":"s1","configId":"thinking","value":true,"type":"boolean"})
        );
    }

    // ── the session's own result ───────────────────────────────────────

    #[test]
    fn session_events_read_the_mode_and_model_out_of_config_options() {
        let result = parse(
            r#"{"sessionId":"sess_1",
                "modes":{"currentModeId":"architect","availableModes":[]},
                "configOptions":[
                  {"id":"model","name":"Model","category":"model","type":"select",
                   "currentValue":"opus","options":[{"value":"opus","name":"Opus"}]}]}"#,
        );
        let events = session_events("sess_1", &result);
        let AgentEvent::SessionStarted {
            session_id,
            model,
            mode,
            tools,
            agents,
        } = &events[0]
        else {
            panic!("expected a session to start, got {events:?}");
        };
        assert_eq!(session_id.as_deref(), Some("sess_1"));
        assert_eq!(model.as_deref(), Some("opus"));
        assert_eq!(mode.as_deref(), Some("architect"));
        assert!(tools.is_empty() && agents.is_empty());
        assert!(
            matches!(events[1], AgentEvent::ConfigOptionUpdate { .. }),
            "expected a config option update, got {events:?}"
        );
    }

    /// With no `modes` block, the mode comes from the config option whose
    /// category says so — the successor mechanism.
    #[test]
    fn session_events_fall_back_to_the_mode_category() {
        let result = parse(
            r#"{"sessionId":"sess_2","configOptions":[
                  {"id":"mode","name":"Mode","category":"mode","type":"select",
                   "currentValue":"code","options":[]}]}"#,
        );
        let AgentEvent::SessionStarted { mode, .. } = &session_events("sess_2", &result)[0] else {
            panic!("expected a session to start");
        };
        assert_eq!(mode.as_deref(), Some("code"));
    }

    #[test]
    fn a_session_with_no_config_options_emits_only_the_start() {
        let result = parse(r#"{"sessionId":"sess_3"}"#);
        assert_eq!(session_events("sess_3", &result).len(), 1);
    }

    // ── prompt content ─────────────────────────────────────────────────

    #[test]
    fn a_text_prompt_is_a_single_text_block() {
        let blocks = prompt_blocks(&[Content::text("do the thing")], PromptCaps::default());
        assert_eq!(blocks, vec![json!({"type":"text","text":"do the thing"})]);
    }

    /// The client adapts content the agent cannot accept, per §8 — sending an
    /// unadvertised kind earns a rejected turn.
    #[test]
    fn an_image_is_down_converted_when_the_agent_does_not_advertise_one() {
        let image = Content::Image {
            data: "AAAA".to_string(),
            mime_type: "image/png".to_string(),
            uri: None,
        };
        let without = prompt_blocks(std::slice::from_ref(&image), PromptCaps::default());
        assert_eq!(without[0].get("type").and_then(Value::as_str), Some("text"));

        let with = prompt_blocks(
            std::slice::from_ref(&image),
            PromptCaps {
                image: true,
                ..PromptCaps::default()
            },
        );
        assert_eq!(
            with,
            vec![json!({"type":"image","data":"AAAA","mimeType":"image/png"})]
        );
    }

    #[test]
    fn prompt_capabilities_default_to_unsupported() {
        assert_eq!(prompt_caps(&Value::Null), PromptCaps::default());
        let caps = prompt_caps(&parse(
            r#"{"promptCapabilities":{"image":true,"audio":false,"embeddedContext":true}}"#,
        ));
        assert_eq!(
            caps,
            PromptCaps {
                image: true,
                audio: false,
                embedded_context: true,
            }
        );
    }

    #[test]
    fn auth_method_ids_are_named_so_a_refusal_is_diagnosable() {
        let init = parse(
            r#"{"protocolVersion":1,
                "authMethods":[{"id":"oauth","name":"Log in"},{"id":"api-key","name":"API key"}]}"#,
        );
        assert_eq!(auth_method_ids(&init), vec!["oauth", "api-key"]);
        assert!(auth_method_ids(&parse(r#"{"authMethods":[]}"#)).is_empty());
        assert!(auth_method_ids(&parse(r#"{}"#)).is_empty());
    }

    /// Fix 8: without the discriminator the number `7` and the string `"7"`
    /// render the same key, and one ask evicts the other.
    #[test]
    fn a_json_rpc_id_key_keeps_a_number_and_a_string_apart() {
        assert_eq!(request_id_of(&json!(4)), "n:4");
        assert_eq!(request_id_of(&json!("req-4")), "s:req-4");
        assert_ne!(request_id_of(&json!(7)), request_id_of(&json!("7")));
    }

    // ── the writer thread, and never blocking the reader ───────────────

    /// Fix 1. Nothing drains the write queue here, so an inline `writeln!`
    /// of this reply would block once it filled the 64 KiB pipe buffer — the
    /// deadlock, since the blocked thread is the one draining stdout. An
    /// enqueue returns whatever the size.
    #[test]
    fn a_large_fs_read_reply_is_enqueued_rather_than_written_inline() {
        let root = temp_root("enqueue");
        let path = root.join("big.txt");
        let text = "a line of quite ordinary length\n".repeat(40_000);
        assert!(text.len() > 1_000_000);
        std::fs::write(&path, &text).unwrap();

        let (shared, written, _events) = test_shared(&root);
        let request = json!({"jsonrpc":"2.0","id":12,"method":"fs/read_text_file",
                             "params":{"sessionId":"s1","path": path.display().to_string()}});
        assert!(serve_request(
            &shared,
            json!(12),
            "fs/read_text_file",
            &request,
            &raw_of(&request)
        ));

        let frames = frames(&written);
        assert_eq!(frames.len(), 1, "frames were: {frames:?}");
        assert_eq!(frames[0]["id"], json!(12));
        assert_eq!(frames[0]["result"]["content"].as_str(), Some(text.as_str()));

        std::fs::remove_dir_all(&root).unwrap();
    }

    // ── every inbound request gets exactly one reply ───────────────────

    /// Fix 2. No entry is recorded, so no `AnswerPermission` could ever reach
    /// this ask; leaving it unanswered blocks the agent forever.
    #[test]
    fn a_permission_ask_is_cancelled_when_the_outstanding_table_is_poisoned() {
        let root = temp_root("poisoned");
        let (shared, written, _events) = test_shared(&root);

        let poisoner = Arc::clone(&shared);
        let _ = std::thread::spawn(move || {
            let _held = poisoner.outstanding.lock().unwrap();
            panic!("poison the outstanding table on purpose");
        })
        .join();
        assert!(
            shared.outstanding.lock().is_err(),
            "expected a poisoned lock"
        );

        let request = json!({"jsonrpc":"2.0","id":5,"method":"session/request_permission",
                             "params":{"options":[]}});
        assert!(serve_request(
            &shared,
            json!(5),
            "session/request_permission",
            &request,
            &raw_of(&request)
        ));
        assert_eq!(
            frames(&written),
            vec![permission_reply(&json!(5), &PermissionOutcome::Cancelled)]
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Fix 2. The event reached nobody, so nobody can answer it.
    #[test]
    fn a_permission_ask_is_cancelled_when_the_event_reaches_nobody() {
        let root = temp_root("noreceiver");
        let (shared, written, events) = test_shared(&root);
        drop(events);

        let request = json!({"jsonrpc":"2.0","id":"ask-1","method":"session/request_permission",
                             "params":{"options":[]}});
        assert!(
            !serve_request(
                &shared,
                json!("ask-1"),
                "session/request_permission",
                &request,
                &raw_of(&request)
            ),
            "with no consumer left the reader stops"
        );
        assert_eq!(
            frames(&written),
            vec![permission_reply(
                &json!("ask-1"),
                &PermissionOutcome::Cancelled
            )]
        );
        assert!(
            shared.outstanding.lock().unwrap().is_empty(),
            "an answered ask must not stay parked"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Fix 2. A reused raw id orphans the ask already parked under it, which
    /// the table cannot represent twice — so the orphan is answered at once.
    #[test]
    fn a_reused_permission_id_cancels_the_ask_it_orphans() {
        let root = temp_root("dupeid");
        let (shared, written, _events) = test_shared(&root);
        let request = json!({"jsonrpc":"2.0","id":5,"method":"session/request_permission",
                             "params":{"options":[]}});

        for _ in 0..2 {
            assert!(serve_request(
                &shared,
                json!(5),
                "session/request_permission",
                &request,
                &raw_of(&request)
            ));
        }

        assert_eq!(
            frames(&written),
            vec![permission_reply(&json!(5), &PermissionOutcome::Cancelled)],
            "the first ask is cancelled, the second stays parked"
        );
        let table = shared.outstanding.lock().unwrap();
        assert_eq!(table.len(), 1);
        assert_eq!(table.get("n:5"), Some(&json!(5)));
        drop(table);

        std::fs::remove_dir_all(&root).unwrap();
    }

    // ── the outstanding table's lifetime is the turn's ─────────────────

    /// Fix 3. An agent may end a turn without waiting for a parked answer.
    /// Those ids are retired: writing a `result` for one is the protocol
    /// violation, so they are dropped in silence — while a cancel, where the
    /// agent *is* still waiting, answers every one of them.
    #[test]
    fn a_turn_ending_response_forgets_parked_permissions_without_answering_them() {
        let root = temp_root("turnend");
        let (shared, written, events) = test_shared(&root);
        shared
            .outstanding
            .lock()
            .unwrap()
            .insert("n:9".to_string(), json!(9));
        *shared.turn.lock().unwrap() = Some(4);

        let response = parse(r#"{"jsonrpc":"2.0","id":4,"result":{"stopReason":"cancelled"}}"#);
        assert!(deliver_response(
            &shared,
            &json!(4),
            &response,
            &raw_of(&response)
        ));

        assert!(shared.outstanding.lock().unwrap().is_empty());
        assert!(
            frames(&written).is_empty(),
            "a retired id must not be answered"
        );
        assert!(matches!(
            events.recv(),
            Ok(Some((AgentEvent::TurnEnded { .. }, _)))
        ));

        // The contrast, stated as a test so the two drains cannot be merged.
        shared
            .outstanding
            .lock()
            .unwrap()
            .insert("n:9".to_string(), json!(9));
        cancel_outstanding(&shared);
        assert_eq!(
            frames(&written),
            vec![permission_reply(&json!(9), &PermissionOutcome::Cancelled)]
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    // ── one turn at a time ─────────────────────────────────────────────

    /// Fix 4. A second prompt used to overwrite the turn slot, stranding the
    /// first response with nothing to match: its caller then waited for a
    /// `TurnEnded` that could never come.
    #[test]
    fn a_second_prompt_is_refused_while_a_turn_is_in_flight() {
        let root = temp_root("twoturns");
        let (shared, written, _events) = test_shared(&root);

        write_input(
            &shared,
            AgentInput::Prompt {
                content: vec![Content::text("one")],
            },
        )
        .unwrap();
        assert_eq!(*shared.turn.lock().unwrap(), Some(1));

        let err = write_input(
            &shared,
            AgentInput::Prompt {
                content: vec![Content::text("two")],
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("id 1"), "error was: {err}");
        assert_eq!(
            frames(&written).len(),
            1,
            "the refused prompt writes no frame"
        );
        assert_eq!(
            *shared.turn.lock().unwrap(),
            Some(1),
            "the in-flight turn survives the refusal"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Fix 4's other half: only a failed *enqueue* un-records the turn.
    #[test]
    fn a_prompt_that_cannot_be_enqueued_leaves_no_turn_on_record() {
        let root = temp_root("nowriter");
        let (shared, written, _events) = test_shared(&root);
        drop(written);

        write_input(
            &shared,
            AgentInput::Prompt {
                content: vec![Content::text("one")],
            },
        )
        .unwrap_err();
        assert_eq!(*shared.turn.lock().unwrap(), None);

        std::fs::remove_dir_all(&root).unwrap();
    }

    // ── the reader's exit ──────────────────────────────────────────────

    /// Fix 7. The pending `Sender`s live in `Shared`, so nothing else ever
    /// drops them: without this drain a waiter burns its whole timeout, and
    /// an agent that dies right after `initialize` costs the caller the full
    /// `HANDSHAKE_TIMEOUT`.
    #[test]
    fn the_readers_exit_fails_every_pending_request_at_once() {
        let root = temp_root("pending");
        let (shared, _written, events) = test_shared(&root);
        let (tx, rx) = mpsc::channel::<Value>();
        shared.pending.lock().unwrap().insert(1, tx);
        shared
            .outstanding
            .lock()
            .unwrap()
            .insert("n:9".to_string(), json!(9));

        drop(ReaderExit(Arc::clone(&shared)));

        assert!(matches!(
            rx.recv_timeout(Duration::from_millis(200)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
        assert!(
            shared.outstanding.lock().unwrap().is_empty(),
            "the agent is gone; the asks go with it"
        );
        assert!(matches!(events.recv(), Ok(None)), "the EOF sentinel");

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Fix 5. The sentinel is the only EOF signal there is, so it has to
    /// survive an unwinding reader — and be sent exactly once.
    #[test]
    fn the_eof_sentinel_survives_a_panicking_reader() {
        let root = temp_root("unwind");
        let (shared, _written, events) = test_shared(&root);

        let guarded = Arc::clone(&shared);
        let _ = std::thread::spawn(move || {
            let _exit = ReaderExit(guarded);
            panic!("the reader fell over on purpose");
        })
        .join();

        assert!(matches!(events.recv(), Ok(None)));
        assert!(events.try_recv().is_err(), "the sentinel is sent once");

        std::fs::remove_dir_all(&root).unwrap();
    }
}
