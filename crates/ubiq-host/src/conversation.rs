//! One live agent: the bridge, the thread that pumps it, and the single
//! mapping between the library's vocabulary and the bus's.
//!
//! **This file is the only place that knows both.** The library speaks
//! `agent_manager::io::AgentEvent`; the wire speaks
//! `ubiq_proto::conversation::ConvUpdate`. Both are the Agent Client
//! Protocol's `session/update` vocabulary, so the mapping is a rename — but
//! it is a rename that happens exactly once, in the host, which is what keeps
//! the interface free of any dependency on the harness library.
//!
//! # The pump
//!
//! `IoBridge::next_event` blocks and both its methods take `&mut self`, so
//! the thread reading a bridge cannot also be handed a prompt. The bridge is
//! therefore owned by its pump thread and prompts arrive through the detached
//! `AgentInputSink` the bridge handed out — a harness that has no such handle
//! answers `None`, and that `None` is the honest signal that it takes no
//! second turn.
//!
//! Events reach the window through a pre-addressed `Mailbox`, the same
//! unbounded path a pseudo-terminal's reader uses, so a window that has
//! fallen behind never stalls the harness.
//!
//! # Identity
//!
//! An `AgentEvent` carries no session id, deliberately: identity belongs to
//! whoever is multiplexing. Here that is the `agent_id` this module stamps on
//! every message — the same role a `sessionId` plays in ACP, one layer up.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::SystemTime;

use agent_manager::io::{
    AgentEvent, AgentInput, AgentInputSink, AgentKill, Content, IoBridge, PermissionOutcome,
};
use ubiq_proto::bus::Mailbox;
use ubiq_proto::conversation::{
    ConfigCategory, ConfigChoice, ConfigOption, ConfigValue, ConvContent, ConvUpdate,
    PermissionKind, PermissionOption, PlanEntry, PlanPriority, PlanStatus, RateLimitRecord,
    StopReason, Subagent, TokenSpend, ToolCallPatch, ToolCallRecord, ToolContent, ToolKind,
    ToolLocation, ToolStatus, UsageRecord,
};
use ubiq_proto::messages::Message;
use ubiq_proto::stats::UsageRow;
use ubiq_proto::work::AgentId;

use crate::store::usage::Usage;

/// Where this conversation's spend is filed, carried by the pump.
///
/// **The meter is written from the pump thread rather than from the coordinator, because the
/// coordinator never sees a usage report.** A pump sends straight to the window's mailbox; nothing
/// on that path comes back through the coordinator's own loop, so the only place a
/// `ConvUpdate::Usage` and the launch's dimensions are both in hand is here. The three dimensions
/// below are facts of the launch, not of any one report, so they are resolved once at
/// [`Conversation::start`] and copied into every row.
///
/// ponytail: one SQLite upsert on the pump thread per usage report — a handful per turn, behind
/// the meter's own mutex in WAL mode. A queue and a writer thread if a harness ever reports spend
/// often enough for that to show.
#[derive(Clone)]
pub struct UsageMeter {
    /// The meter itself. Shared: every live conversation writes into the one database.
    pub meter: Arc<Usage>,
    /// The project's ULID, empty when the work belonged to no project.
    pub project: String,
    /// The agent type: `claude-code`, `codex`, and the rest.
    pub harness: String,
    /// Empty when the harness ran as its own default identity.
    pub account: String,
}

/// A running conversation, as the coordinator holds it.
///
/// It owns no bridge: the pump thread does. What is left here is the way in —
/// and the way to stop, which is the same thing, because closing a harness's
/// input is what makes it exit.
pub struct Conversation {
    id: AgentId,
    input: Option<Arc<dyn AgentInputSink>>,
    /// The way out that does not ask: kills the harness's process outright. `None` for a bridge
    /// that names no process. Taken at [`Conversation::start`] for the same reason `input` is —
    /// the pump thread owns the bridge, so nothing else can reach the child through it.
    kill: Option<Arc<dyn AgentKill>>,
    pump: Option<thread::JoinHandle<()>>,
    /// Set by the pump, just before it returns, whether the harness ended on its own or the
    /// window went first. The coordinator polls this — the same shape it already polls
    /// `active_searches`'s own "is this over" flag — to reap a conversation whose harness quit
    /// without anybody asking it to.
    ended: Arc<AtomicBool>,
    /// The pump's own sequence counter, published after every send, so an unload can hand the
    /// coordinator the last `seq` it reached without racing the pump for it.
    seq: Arc<AtomicU64>,
    /// Set by `stop(true)` before the pump is asked to exit, or at [`Conversation::start`] for a
    /// harness that ends every turn by exiting. Read by the pump on its way out to decide whether
    /// to send the final `ConversationEnded` — an unload wants exactly one lifecycle message
    /// (`ConversationUnloaded`), not that plus this.
    ///
    /// **A one-shot harness sets it before the pump starts, not at reap.** Its process ends
    /// unasked, so by the time the coordinator polls `ended()` the pump has already decided
    /// whether to speak; there is no later moment at which setting the flag would still be read.
    quiet: Arc<AtomicBool>,
    /// The harness's own session id, as its `SessionStarted` reported it. Written by the pump,
    /// read by the coordinator — the same `Mutex`-over-a-shared-fact shape `outstanding` uses,
    /// and for the same reason: the pump is what sees it and the coordinator is what needs it.
    ///
    /// It is the whole of what continues a one-shot conversation: the next turn is a fresh
    /// process launched with this id as its resume.
    session: Arc<Mutex<Option<String>>>,
    /// Every permission request this harness is still waiting on, in arrival order.
    ///
    /// **Shared with the pump, because the pump is what sees a request and the coordinator is
    /// what answers one.** Upstream requires a cancelling client to answer every outstanding
    /// request with `cancelled`, and there is no other place both halves can agree on which those
    /// are. A `Mutex` rather than a channel: the only operations are "one arrived", "one was
    /// answered" and "take them all", and every one of them is a handful of strings.
    outstanding: Arc<Mutex<Vec<String>>>,
    /// The agent's first message, whole, once the turn that carried it has ended.
    ///
    /// Written by the pump, read by the coordinator — the same shape [`Self::session`] uses, and
    /// for the same reason: the pump is what sees a message and the coordinator is what has the
    /// backend that can name one. It is the *answered* half of the opening exchange; the asked
    /// half never leaves the coordinator, which is where a prompt arrives in the first place.
    ///
    /// `None` until the first turn ends. Written exactly once, so a conversation that goes on
    /// talking does not keep re-offering itself to be named.
    first_reply: Arc<Mutex<Option<String>>>,
}

/// The agent's first message, while the pump is still gathering it.
///
/// A message arrives as chunks that share a `message_id`, so "the first message" is an
/// accumulation and not an event — and a name written from whichever chunk happened to arrive
/// first would be a name written from half a sentence.
struct Gathering {
    /// The id the chunks being gathered share. A different one is a *second* message, which this
    /// pass has no interest in.
    message_id: Option<String>,
    text: String,
}

impl Conversation {
    /// Start pumping `bridge` onto `out`, stamping every message with `id`.
    ///
    /// `start_seq` is where the sequence counter picks up rather than always zero, so a
    /// conversation that said something before this harness existed — P3's pending picker, over
    /// `ConversationUpdate` — and the harness's own first frame are one unbroken sequence.
    ///
    /// `quiet` set means this process ending is a *turn* ending, not the conversation's: the pump
    /// says nothing on its way out and the coordinator decides what that means. It is what a
    /// one-shot harness is started with, because a `ConversationEnded` after every answer is
    /// exactly what makes such a harness read as dead.
    pub fn start(
        id: AgentId,
        bridge: Box<dyn IoBridge>,
        out: Mailbox,
        start_seq: u64,
        usage: Option<UsageMeter>,
        quiet: bool,
    ) -> Self {
        let input = bridge.input();
        let kill = bridge.killer();
        let ended = Arc::new(AtomicBool::new(false));
        let seq = Arc::new(AtomicU64::new(start_seq));
        let quiet = Arc::new(AtomicBool::new(quiet));
        let outstanding = Arc::new(Mutex::new(Vec::new()));
        let session = Arc::new(Mutex::new(None));
        let pump_ended = ended.clone();
        let pump_seq = seq.clone();
        let pump_quiet = quiet.clone();
        let pump_outstanding = outstanding.clone();
        let pump_session = session.clone();
        let first_reply = Arc::new(Mutex::new(None));
        let pump_first_reply = first_reply.clone();
        let pump = thread::Builder::new()
            .name(format!("agent-{id}"))
            .spawn(move || {
                pump(
                    id,
                    bridge,
                    out,
                    start_seq,
                    pump_ended,
                    pump_seq,
                    pump_quiet,
                    pump_outstanding,
                    pump_session,
                    pump_first_reply,
                    usage,
                )
            })
            .ok();

        Self {
            id,
            input,
            kill,
            pump,
            ended,
            seq,
            quiet,
            outstanding,
            session,
            first_reply,
        }
    }

    /// The harness's own session id, once it has named one. `None` until its `SessionStarted`
    /// arrives, and for a harness that names none at all — a conversation with no id to resume
    /// from cannot be continued by relaunching, which is a fact worth reading as `None` rather
    /// than guessing around.
    pub fn session_id(&self) -> Option<String> {
        self.session.lock().ok().and_then(|held| held.clone())
    }

    /// The agent's first message, once the turn that carried it has ended.
    ///
    /// Cloned rather than taken: what stops a conversation being named twice is the coordinator's
    /// own record of having asked, not the emptying of this slot — a naming that the model refused
    /// must not come back round on the next poll.
    pub fn first_reply(&self) -> Option<String> {
        self.first_reply.lock().ok().and_then(|held| held.clone())
    }

    /// Whether the harness behind this conversation has ended by itself — its pump thread has
    /// returned, or is on its way out. Checked by the coordinator's own reap, which then does
    /// exactly what an explicit `EndConversation` does; that removal is idempotent, so this is
    /// safe to check even while a request to end the same conversation is racing it.
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::Relaxed)
    }

    /// Whether this harness accepts anything after its first turn. A composer
    /// asks before offering to send, rather than discovering it by sending
    /// into a void.
    pub fn accepts_input(&self) -> bool {
        self.input.is_some()
    }

    /// Send one turn.
    pub fn prompt(&self, text: String) -> anyhow::Result<()> {
        self.send(AgentInput::Prompt {
            content: vec![Content::text(text)],
        })
    }

    /// Interrupt the turn in flight, and nothing else: **the conversation and its harness stay**,
    /// and the next [`Conversation::prompt`] reaches the same agent. Ending the harness is
    /// [`Conversation::stop`].
    ///
    /// **Every request still waiting is answered as cancelled first.** Upstream makes that the
    /// cancelling client's obligation, and it is not a courtesy: a harness holding an unanswered
    /// request has no timeout to fall back on, so the turn it belongs to would never end. A
    /// refused answer is logged rather than returned — the cancel itself is what the caller asked
    /// for, and a harness that would not take the answer is already on its way out.
    pub fn cancel(&self) -> anyhow::Result<()> {
        self.answer_outstanding_as_cancelled();
        self.send(AgentInput::Cancel)
    }

    /// Answer everything still waiting as cancelled — shared by [`Conversation::cancel`] and
    /// [`Conversation::stop`], because both leave the turn and neither may leave a harness
    /// holding an ask nothing will ever answer.
    fn answer_outstanding_as_cancelled(&self) {
        for request_id in self.take_outstanding() {
            if let Err(error) = self.send(AgentInput::AnswerPermission {
                request_id,
                outcome: PermissionOutcome::Cancelled,
                updated_input: None,
            }) {
                tracing::debug!(agent = %self.id, %error, "a cancelled permission went unanswered");
            }
        }
    }

    /// Answer a permission request by naming one of the options it offered.
    ///
    /// `updated_input` is always `None`: the response carries an option id and nothing else, so
    /// there is no editing of a tool's input on this path and no place to put one.
    pub fn answer_permission(&self, request_id: String, option_id: String) -> anyhow::Result<()> {
        self.forget_outstanding(&request_id);
        self.send(AgentInput::AnswerPermission {
            request_id,
            outcome: PermissionOutcome::Selected { option_id },
            updated_input: None,
        })
    }

    /// Everything still waiting, emptied — a request answered once must not be answered twice.
    fn take_outstanding(&self) -> Vec<String> {
        match self.outstanding.lock() {
            Ok(mut held) => std::mem::take(&mut *held),
            // A poisoned lock means the pump panicked mid-record. The list is of no further use
            // and this harness is finished; cancelling it is still worth doing.
            Err(_) => Vec::new(),
        }
    }

    fn forget_outstanding(&self, request_id: &str) {
        if let Ok(mut held) = self.outstanding.lock() {
            held.retain(|held| held.as_str() != request_id);
        }
    }

    /// Change a model, a mode or a thinking level.
    pub fn set_config(&self, config_id: String, value: String) -> anyhow::Result<()> {
        self.send(AgentInput::SetConfigOption {
            config_id,
            value: agent_manager::io::ConfigSetting::Text(value),
        })
    }

    fn send(&self, input: AgentInput) -> anyhow::Result<()> {
        let sink = self
            .input
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("this harness takes no input after it is launched"))?;
        sink.send(input)
    }

    /// Stop the harness and wait for its pump to finish, returning the last `seq` it reached.
    ///
    /// [`AgentInput::Shutdown`] closes the child's input, which is what makes it exit; the
    /// bridge's own teardown then gives it a bounded window to drain before
    /// killing it. So the wait here is for a thread that is already ending
    /// rather than one that has to be interrupted. **This is the teardown, not
    /// [`Conversation::cancel`]** — a cancel interrupts the turn and keeps the harness, so a stop
    /// that reused it would leave the process running.
    ///
    /// `quiet` set is an unload: the pump skips its final `ConversationEnded` so the coordinator's
    /// own `ConversationUnloaded` is the only lifecycle message this stop produces.
    pub fn stop(mut self, quiet: bool) -> u64 {
        self.quiet.store(quiet, Ordering::Relaxed);
        self.answer_outstanding_as_cancelled();
        let _ = self.send(AgentInput::Shutdown);
        if let Some(pump) = self.pump.take() {
            let _ = pump.join();
        }
        tracing::debug!(agent = %self.id, "conversation stopped");
        self.seq.load(Ordering::Relaxed)
    }

    /// Kill the harness now and then reap it, returning the last `seq` its pump reached.
    ///
    /// The forceful twin of [`Self::stop`], and the difference is the order: `stop` asks the
    /// harness to shut down and then waits, so a harness that does not act on the ask holds the
    /// caller for the bridge's whole grace window. This kills the process first, so the stream
    /// the pump is blocked on hits end-of-file straight away and the join that follows is for a
    /// thread already on its way out. Nothing is asked and no outstanding permission is answered
    /// — there is no process left to answer to.
    ///
    /// Always quiet: the pump skips its own `ConversationEnded`, on the same terms as an unload,
    /// because the coordinator's `ConversationUnloaded` is the one lifecycle message an abort
    /// produces. A bridge with no killer degrades to exactly what `stop(true)` does.
    pub fn abort(mut self) -> u64 {
        self.quiet.store(true, Ordering::Relaxed);
        match &self.kill {
            Some(kill) => {
                if let Err(error) = kill.kill() {
                    tracing::warn!(agent = %self.id, %error, "the harness would not be killed");
                }
            }
            // Asking is all that is left to unblock the pump — which is what `stop` does.
            None => {
                let _ = self.send(AgentInput::Shutdown);
            }
        }
        if let Some(pump) = self.pump.take() {
            let _ = pump.join();
        }
        tracing::debug!(agent = %self.id, "conversation aborted");
        self.seq.load(Ordering::Relaxed)
    }
}

/// The pump thread: read the bridge until it ends, and put everything it says
/// on the bus.
// One thread entry point called from exactly one place; a struct to carry its arguments would be
// a name for the argument list and nothing else.
#[allow(clippy::too_many_arguments)]
fn pump(
    id: AgentId,
    mut bridge: Box<dyn IoBridge>,
    out: Mailbox,
    start_seq: u64,
    ended: Arc<AtomicBool>,
    seq_counter: Arc<AtomicU64>,
    quiet: Arc<AtomicBool>,
    outstanding: Arc<Mutex<Vec<String>>>,
    session: Arc<Mutex<Option<String>>>,
    first_reply: Arc<Mutex<Option<String>>>,
    usage: Option<UsageMeter>,
) {
    let mut seq = start_seq;
    let mut stop_reason = StopReason::EndTurn;
    // The opening reply, until the turn carrying it ends and it is published. `None` after that,
    // which is also what it is for every turn after the first: this pass runs once.
    let mut gathering: Option<Gathering> = None;
    let mut gathered = false;

    loop {
        let (event, raw) = match bridge.next_event_raw() {
            Ok(Some(framed)) => framed,
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(agent = %id, %error, "conversation stream failed");
                out.send(Message::ConversationError {
                    agent_id: id,
                    error: error.to_string(),
                });
                stop_reason = StopReason::Failed;
                break;
            }
        };

        // The last turn's reason is the conversation's, since a harness that
        // ends after a failed turn ended because of it.
        if let AgentEvent::TurnEnded { stop_reason: r, .. } = &event {
            stop_reason = map_stop_reason(r);
            // Nothing is waiting on an answer once the turn is over, and a stale id would make a
            // later cancel answer a request that closed with the turn.
            if let Ok(mut held) = outstanding.lock() {
                held.clear();
            }
        }

        // Recorded rather than merely forwarded: for a one-shot harness this id is the only
        // thread between one turn's process and the next's, and the coordinator has no other
        // sight of it — every `ConvUpdate` goes straight to the window.
        if let AgentEvent::SessionStarted { session_id, .. } = &event
            && let Some(session_id) = session_id
            && let Ok(mut held) = session.lock()
        {
            *held = Some(session_id.clone());
        }

        // Written before the update goes out, so a cancel that arrives the instant the window
        // draws the prompt still finds the request to answer. Read by `Conversation::cancel`.
        if let AgentEvent::PermissionRequest { request_id, .. } = &event
            && let Ok(mut held) = outstanding.lock()
            && !held.contains(request_id)
        {
            held.push(request_id.clone());
        }

        // The answered half of the opening exchange, gathered for the naming pass the
        // coordinator runs. A subagent's prose is skipped: what a conversation is *about* is what
        // the agent the user is talking to said, not what something it spawned reported back.
        if !gathered {
            match &event {
                AgentEvent::AgentMessageChunk {
                    content: Content::Text { text },
                    message_id,
                    origin,
                } if origin.parent_tool_use_id.is_none() => match &mut gathering {
                    Some(held) if held.message_id == *message_id => held.text.push_str(text),
                    // A different id is a second message, and the first one is what this wants.
                    Some(_) => {}
                    None => {
                        gathering = Some(Gathering {
                            message_id: message_id.clone(),
                            text: text.clone(),
                        });
                    }
                },
                // Published at the end of the turn rather than at the end of the message,
                // because a turn is when the agent has finished answering and is also when the
                // coordinator is free to ask a model about it. A one-shot harness exits straight
                // after this, so the coordinator's own loop takes the naming before it reaps the
                // conversation — see `Coordinator::name_conversations`.
                AgentEvent::TurnEnded { .. } => {
                    gathered = true;
                    if let Some(held) = gathering.take()
                        && !held.text.trim().is_empty()
                        && let Ok(mut slot) = first_reply.lock()
                    {
                        *slot = Some(held.text);
                    }
                }
                _ => {}
            }
        }

        let Some(update) = map_event(event) else {
            continue;
        };

        // Read before the send, which takes the update; recorded after it, so the window is
        // never made to wait on the meter.
        let row = match (&usage, &update) {
            (Some(meter), ConvUpdate::Usage(record)) => usage_row(meter, record),
            _ => None,
        };

        seq += 1;
        seq_counter.store(seq, Ordering::Relaxed);
        tracing::debug!(agent = %id, seq, update = ?update, "conversation update");
        let listening = out.send(Message::ConversationUpdate {
            agent_id: id,
            seq,
            update: Box::new(update),
            raw,
        });

        // A meter that refuses is logged and dropped, on the same bargain `Usage::open` makes: a
        // read-only config root costs the user their token history, not their session.
        if let (Some(meter), Some(row)) = (&usage, row)
            && let Err(error) = meter.meter.record(SystemTime::now(), &row)
        {
            tracing::warn!(agent = %id, "the usage meter refused a record: {error}");
        }

        if !listening {
            // The window this agent belongs to has gone. Nothing left to say.
            tracing::debug!(agent = %id, "conversation has no listener; pump ending");
            ended.store(true, Ordering::Relaxed);
            return;
        }
    }

    // Set before the last send: the coordinator's reap and this send race harmlessly (removal
    // is idempotent), but there must be no window where the pump has already returned — the
    // thread `Conversation::stop` would join — while the flag still reads false.
    ended.store(true, Ordering::Relaxed);
    if !quiet.load(Ordering::Relaxed) {
        out.send(Message::ConversationEnded {
            agent_id: id,
            stop_reason,
        });
    }
}

/// The meter row one usage report writes, or `None` when the report is not a spend.
///
/// **Occupancy is a level and a level is never accumulated**, so a report with no `spend` writes
/// nothing at all — it moved the context ring, which the transcript already carries. A subagent's
/// report is a spend row like any other, filed under its own `subagent` so a turn's total splits
/// between the conversation and the agents it spawned instead of merging into one bucket.
///
/// `msgs_in`/`msgs_out`/`tool_calls` stay zero. The pump sees chunks and tool-call patches, not
/// the message and call counts a *turn's* spend belongs to, and there is no honest way to
/// attribute the ones it has seen to the report in hand — a guessed count is worse than an
/// absent one.
fn usage_row(meter: &UsageMeter, record: &UsageRecord) -> Option<UsageRow> {
    let spend = record.spend.as_ref()?;
    Some(UsageRow {
        // Ignored by `Usage::record`, which floors the instant it is given, twice.
        bucket: 0,
        project: meter.project.clone(),
        harness: meter.harness.clone(),
        account: meter.account.clone(),
        model: record.model.clone().unwrap_or_default(),
        subagent: record.subagent.clone().unwrap_or_default(),
        tokens_in: spend.input,
        tokens_out: spend.output,
        tokens_think: spend.thinking,
        // Everything counted that is none of the three above: cache read and cache creation are
        // distinct to the harness, and this column is the one place they are not.
        tokens_other: spend.cache_read.saturating_add(spend.cache_creation),
        msgs_in: 0,
        msgs_out: 0,
        tool_calls: 0,
    })
}

/// Who said it, as the transcript needs it: the instance *and* the kind.
///
/// The instance is the whole point — `parent_tool_use_id` is the id of the `Task` call that
/// spawned the agent, so three `general-purpose` subagents in one turn stay three agents instead
/// of collapsing into one interleaved transcript. A line with no parent is the conversation's own,
/// whatever else the origin says.
fn map_origin(origin: agent_manager::io::Origin) -> Option<Subagent> {
    origin.parent_tool_use_id.map(|id| Subagent {
        id,
        kind: origin.subagent_type,
        model: origin.model,
        thinking: origin.thinking,
    })
}

/// The whole of the translation. `None` is an event the wire has no place for
/// yet — a harness log line, which belongs in the diagnostics ring it is
/// already in rather than in a transcript.
fn map_event(event: AgentEvent) -> Option<ConvUpdate> {
    let update = match event {
        AgentEvent::SessionStarted {
            session_id,
            model,
            mode,
            tools,
            agents,
        } => ConvUpdate::Started {
            session_id,
            model,
            mode,
            tools,
            agents,
        },

        AgentEvent::UserMessageChunk {
            content,
            message_id,
        } => ConvUpdate::UserChunk {
            content: map_content(content),
            message_id,
        },
        AgentEvent::AgentMessageChunk {
            content,
            message_id,
            origin,
        } => ConvUpdate::AgentChunk {
            content: map_content(content),
            message_id,
            subagent: map_origin(origin),
        },
        AgentEvent::AgentThoughtChunk {
            content,
            message_id,
            origin,
        } => ConvUpdate::ThoughtChunk {
            content: map_content(content),
            message_id,
            subagent: map_origin(origin),
        },

        AgentEvent::ToolCall { call } => ConvUpdate::ToolCall(ToolCallRecord {
            id: call.id,
            title: call.title,
            kind: map_kind(call.kind),
            status: map_status(call.status),
            content: call.content.into_iter().map(map_tool_content).collect(),
            locations: call.locations.into_iter().map(map_location).collect(),
            subagent: map_origin(call.origin),
        }),
        AgentEvent::ToolCallUpdate { update } => ConvUpdate::ToolCallUpdate(map_patch(update)),

        AgentEvent::Plan { entries } => ConvUpdate::Plan(
            entries
                .into_iter()
                .map(|entry| PlanEntry {
                    content: entry.content,
                    priority: match entry.priority {
                        agent_manager::io::PlanPriority::High => PlanPriority::High,
                        agent_manager::io::PlanPriority::Medium => PlanPriority::Medium,
                        agent_manager::io::PlanPriority::Low => PlanPriority::Low,
                    },
                    status: match entry.status {
                        agent_manager::io::PlanStatus::Pending => PlanStatus::Pending,
                        agent_manager::io::PlanStatus::InProgress => PlanStatus::InProgress,
                        agent_manager::io::PlanStatus::Completed => PlanStatus::Completed,
                    },
                })
                .collect(),
        ),

        AgentEvent::ConfigOptionUpdate { options } => {
            ConvUpdate::ConfigOptions(options.into_iter().map(map_config).collect())
        }
        AgentEvent::CurrentModeUpdate { current_mode_id } => ConvUpdate::ModeChanged {
            mode_id: current_mode_id,
        },
        AgentEvent::SessionInfoUpdate { title, .. } => ConvUpdate::Title(title?),

        AgentEvent::UsageUpdate {
            used,
            size,
            cost,
            model,
            spend,
            origin,
        } => ConvUpdate::Usage(UsageRecord {
            used,
            size,
            cost_usd: cost.map(|cost| cost.amount),
            model,
            spend: spend.map(|spend| TokenSpend {
                input: spend.input,
                output: spend.output,
                thinking: spend.thinking,
                cache_read: spend.cache_read,
                cache_creation: spend.cache_creation,
            }),
            subagent: origin.subagent_type,
        }),

        AgentEvent::RateLimitUpdate {
            five_hour,
            seven_day,
            status,
            overage_status,
            overage_reason,
        } => ConvUpdate::RateLimit(RateLimitRecord {
            five_hour_pct: five_hour.as_ref().map(|w| w.utilization_pct),
            five_hour_resets_at: five_hour.as_ref().map(|w| w.resets_at),
            seven_day_pct: seven_day.as_ref().map(|w| w.utilization_pct),
            seven_day_resets_at: seven_day.as_ref().map(|w| w.resets_at),
            status,
            overage_status,
            overage_reason,
        }),

        AgentEvent::PermissionRequest {
            request_id,
            tool_call,
            options,
        } => ConvUpdate::PermissionRequest {
            request_id,
            tool_call: map_patch(tool_call),
            options: options
                .into_iter()
                .map(|option| PermissionOption {
                    option_id: option.option_id,
                    name: option.name,
                    kind: match option.kind {
                        agent_manager::io::PermissionKind::AllowOnce => PermissionKind::AllowOnce,
                        agent_manager::io::PermissionKind::AllowAlways => {
                            PermissionKind::AllowAlways
                        }
                        agent_manager::io::PermissionKind::RejectOnce => PermissionKind::RejectOnce,
                        agent_manager::io::PermissionKind::RejectAlways => {
                            PermissionKind::RejectAlways
                        }
                    },
                })
                .collect(),
        },

        AgentEvent::TurnEnded { stop_reason, error } => ConvUpdate::TurnEnded {
            stop_reason: map_stop_reason(&stop_reason),
            error,
        },

        // Already in the diagnostics ring, under the harness's own subsystem.
        AgentEvent::AvailableCommandsUpdate { .. } | AgentEvent::Log { .. } => return None,
    };
    Some(update)
}

fn map_content(content: Content) -> ConvContent {
    match content {
        Content::Text { text } => ConvContent::Text(text),
        Content::Image { mime_type, .. } => ConvContent::Other {
            kind: "image".to_string(),
            description: mime_type,
        },
        Content::Audio { mime_type, .. } => ConvContent::Other {
            kind: "audio".to_string(),
            description: mime_type,
        },
        Content::ResourceLink { uri, name, .. } => ConvContent::Other {
            kind: "resource".to_string(),
            description: format!("{name} ({uri})"),
        },
        Content::Resource { .. } => ConvContent::Other {
            kind: "resource".to_string(),
            description: String::new(),
        },
    }
}

fn map_patch(update: agent_manager::io::ToolCallUpdate) -> ToolCallPatch {
    ToolCallPatch {
        id: update.id,
        title: update.title,
        kind: update.kind.map(map_kind),
        status: update.status.map(map_status),
        content: update
            .content
            .map(|items| items.into_iter().map(map_tool_content).collect()),
        locations: update
            .locations
            .map(|items| items.into_iter().map(map_location).collect()),
    }
}

fn map_tool_content(content: agent_manager::io::ToolContent) -> ToolContent {
    use agent_manager::io::ToolContent as Lib;
    match content {
        Lib::Content { content } => match map_content(content) {
            ConvContent::Text(text) => ToolContent::Text(text),
            ConvContent::Other { kind, description } => {
                ToolContent::Text(format!("[{kind}] {description}"))
            }
        },
        Lib::Diff {
            path,
            old_text,
            new_text,
        } => ToolContent::Diff {
            path,
            old_text,
            new_text,
        },
        // A terminal the client was asked to create. Nothing asks yet, and a
        // transcript that claimed one exists would be lying.
        Lib::Terminal { terminal_id } => ToolContent::Text(format!("terminal {terminal_id}")),
    }
}

fn map_location(location: agent_manager::io::ToolLocation) -> ToolLocation {
    ToolLocation {
        path: location.path,
        line: location.line,
    }
}

fn map_kind(kind: agent_manager::io::ToolKind) -> ToolKind {
    use agent_manager::io::ToolKind as Lib;
    match kind {
        Lib::Read => ToolKind::Read,
        Lib::Edit => ToolKind::Edit,
        Lib::Delete => ToolKind::Delete,
        Lib::Move => ToolKind::Move,
        Lib::Search => ToolKind::Search,
        Lib::Execute => ToolKind::Execute,
        Lib::Think => ToolKind::Think,
        Lib::Fetch => ToolKind::Fetch,
        Lib::SwitchMode => ToolKind::SwitchMode,
        Lib::Delegate => ToolKind::Delegate,
        Lib::Other => ToolKind::Other,
    }
}

fn map_status(status: agent_manager::io::ToolStatus) -> ToolStatus {
    use agent_manager::io::ToolStatus as Lib;
    match status {
        Lib::Pending => ToolStatus::Pending,
        Lib::InProgress => ToolStatus::InProgress,
        Lib::Completed => ToolStatus::Completed,
        Lib::Failed => ToolStatus::Failed,
    }
}

fn map_stop_reason(reason: &agent_manager::io::StopReason) -> StopReason {
    use agent_manager::io::StopReason as Lib;
    match reason {
        Lib::EndTurn => StopReason::EndTurn,
        Lib::MaxTokens => StopReason::MaxTokens,
        Lib::MaxTurnRequests => StopReason::MaxTurnRequests,
        Lib::Refusal => StopReason::Refusal,
        Lib::Cancelled => StopReason::Cancelled,
        Lib::Failed => StopReason::Failed,
    }
}

fn map_config(option: agent_manager::io::ConfigOption) -> ConfigOption {
    use agent_manager::io::ConfigCategory as LibCategory;
    use agent_manager::io::ConfigValue as LibValue;
    ConfigOption {
        id: option.id,
        name: option.name,
        description: option.description,
        category: option.category.map(|category| match category {
            LibCategory::Mode => ConfigCategory::Mode,
            LibCategory::Model => ConfigCategory::Model,
            LibCategory::ModelConfig => ConfigCategory::ModelConfig,
            LibCategory::ThoughtLevel => ConfigCategory::ThoughtLevel,
            LibCategory::Other(other) => ConfigCategory::Other(other),
        }),
        value: match option.value {
            LibValue::Select {
                current_value,
                options,
            } => ConfigValue::Select {
                current: current_value,
                choices: options
                    .into_iter()
                    .map(|choice| ConfigChoice {
                        value: choice.value,
                        name: choice.name,
                        description: choice.description,
                        group: choice.group,
                    })
                    .collect(),
            },
            LibValue::Boolean { current_value } => ConfigValue::Flag {
                current: current_value,
            },
        },
    }
}

/// A bridge for tests elsewhere in this crate that need a genuinely live `Conversation` — one
/// with a pump actually blocked in `next_event`, `self.conversations` holding it — without a real
/// harness process. `coordinator.rs`'s own lifecycle tests are what this is for: `unload`/`resume`
/// only touch what a `Conversation` and its owning maps look like, never what a harness says, so a
/// bridge that never says anything is the whole of what they need.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Arc;
    use std::sync::mpsc;

    use agent_manager::io::{AgentEvent, AgentInput, AgentInputSink, IoBridge};

    /// Blocks in `next_event` until shut down, the same way a real child blocks until closing its
    /// input makes it exit.
    pub(crate) struct Idle {
        rx: mpsc::Receiver<()>,
        tx: mpsc::Sender<()>,
    }

    impl Idle {
        pub(crate) fn new() -> Self {
            let (tx, rx) = mpsc::channel();
            Self { tx, rx }
        }
    }

    impl IoBridge for Idle {
        fn send(&mut self, _input: AgentInput) -> anyhow::Result<()> {
            Ok(())
        }

        fn next_event(&mut self) -> anyhow::Result<Option<AgentEvent>> {
            let _ = self.rx.recv();
            Ok(None)
        }

        fn input(&self) -> Option<Arc<dyn AgentInputSink>> {
            Some(Arc::new(IdleInput {
                tx: self.tx.clone(),
            }))
        }
    }

    struct IdleInput {
        tx: mpsc::Sender<()>,
    }

    impl AgentInputSink for IdleInput {
        fn send(&self, input: AgentInput) -> anyhow::Result<()> {
            // A stop is a `Shutdown`; a cancel leaves the harness alive, so only the
            // former ends this fake's stream.
            if matches!(input, AgentInput::Shutdown) {
                let _ = self.tx.send(());
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_manager::io::{ToolCall as LibCall, ToolKind as LibKind, ToolStatus as LibStatus};

    #[test]
    fn a_message_chunk_keeps_its_message_id() {
        let update = map_event(AgentEvent::AgentMessageChunk {
            content: Content::text("hello"),
            message_id: Some("m1".to_string()),
            origin: agent_manager::io::Origin::default(),
        })
        .unwrap();
        assert_eq!(
            update,
            ConvUpdate::AgentChunk {
                content: ConvContent::Text("hello".to_string()),
                message_id: Some("m1".to_string()),
                subagent: None,
            }
        );
    }

    /// A subagent's speech is its own, and the transcript has to be able to say whose it was.
    #[test]
    fn a_subagents_chunk_names_the_subagent() {
        let update = map_event(AgentEvent::AgentMessageChunk {
            content: Content::text("Good day, Marco"),
            message_id: Some("m2".to_string()),
            origin: agent_manager::io::Origin {
                parent_tool_use_id: Some("toolu_015X".to_string()),
                subagent_type: Some("general-purpose".to_string()),
                model: Some("claude-sonnet-5".to_string()),
                thinking: None,
            },
        })
        .unwrap();
        let ConvUpdate::AgentChunk { subagent, .. } = update else {
            panic!("expected an agent chunk");
        };
        assert_eq!(
            subagent,
            Some(Subagent {
                id: "toolu_015X".to_string(),
                kind: Some("general-purpose".to_string()),
                model: Some("claude-sonnet-5".to_string()),
                thinking: None,
            }),
            "the instance is the id and the type, and what it runs as travels with it"
        );
    }

    #[test]
    fn a_tool_call_keeps_its_kind_and_its_diff() {
        let mut call = LibCall::new("t1", "Edit a.rs");
        call.kind = LibKind::Edit;
        call.status = LibStatus::InProgress;
        call.content = vec![agent_manager::io::ToolContent::Diff {
            path: "/tmp/a.rs".to_string(),
            old_text: Some("one".to_string()),
            new_text: "two".to_string(),
        }];

        let update = map_event(AgentEvent::ToolCall { call }).unwrap();
        let ConvUpdate::ToolCall(record) = update else {
            panic!("expected a tool call");
        };
        assert_eq!(record.kind, ToolKind::Edit);
        assert_eq!(record.status, ToolStatus::InProgress);
        assert_eq!(
            record.content,
            vec![ToolContent::Diff {
                path: "/tmp/a.rs".to_string(),
                old_text: Some("one".to_string()),
                new_text: "two".to_string(),
            }]
        );
    }

    /// A patch must arrive as a patch: what it does not name stays `None`, or
    /// applying it would clear a title nobody changed.
    #[test]
    fn a_patch_stays_a_patch() {
        let update = map_event(AgentEvent::ToolCallUpdate {
            update: agent_manager::io::ToolCallUpdate::finished("t1", LibStatus::Completed),
        })
        .unwrap();
        let ConvUpdate::ToolCallUpdate(patch) = update else {
            panic!("expected an update");
        };
        assert_eq!(patch.id, "t1");
        assert_eq!(patch.status, Some(ToolStatus::Completed));
        assert_eq!(patch.title, None);
        assert_eq!(patch.content, None);
    }

    #[test]
    fn usage_crosses_with_the_window_that_makes_it_a_ratio() {
        let update = map_event(AgentEvent::UsageUpdate {
            used: 100,
            size: 200_000,
            cost: Some(agent_manager::io::Cost {
                amount: 0.5,
                currency: "USD".to_string(),
            }),
            model: Some("claude-opus-5".to_string()),
            spend: Some(agent_manager::io::Spend {
                input: 200,
                output: 100,
                thinking: 0,
                cache_read: 900,
                cache_creation: 100,
            }),
            origin: agent_manager::io::Origin::default(),
        })
        .unwrap();
        let ConvUpdate::Usage(usage) = update else {
            panic!("expected usage");
        };
        assert_eq!(usage.size, 200_000);
        assert_eq!(usage.cost_usd, Some(0.5));
        // The expected figures changed with the occupancy/spend split: what used to be one
        // `total_tokens` is now a spend whose parts stay apart, and "cached" is cache *read*.
        assert_eq!(usage.spend.map(|spend| spend.total()), Some(1_300));
        assert_eq!(usage.spend.map(|spend| spend.cached()), Some(900));
        assert_eq!(usage.subagent, None);
        assert_eq!(usage.context_pct(), Some(0));
    }

    #[test]
    fn rate_limit_maps_both_windows_and_the_status() {
        let update = map_event(AgentEvent::RateLimitUpdate {
            overage_status: Some("rejected".to_string()),
            overage_reason: Some("group_zero_credit_limit".to_string()),
            five_hour: Some(agent_manager::io::RateLimitWindow {
                utilization_pct: 7,
                resets_at: 1_788_474_600,
            }),
            seven_day: Some(agent_manager::io::RateLimitWindow {
                utilization_pct: 21,
                resets_at: 1_788_796_800,
            }),
            status: "allowed".to_string(),
        })
        .unwrap();
        let ConvUpdate::RateLimit(record) = update else {
            panic!("expected a rate limit record");
        };
        assert_eq!(record.five_hour_pct, Some(7));
        assert_eq!(record.five_hour_resets_at, Some(1_788_474_600));
        assert_eq!(record.seven_day_pct, Some(21));
        assert_eq!(record.seven_day_resets_at, Some(1_788_796_800));
        assert_eq!(record.status, "allowed");
    }

    /// A harness log line is already in the diagnostics ring; putting it in a
    /// transcript too would say it twice in the place it belongs least.
    #[test]
    fn a_log_line_is_not_a_conversation_update() {
        assert!(
            map_event(AgentEvent::Log {
                level: "info".to_string(),
                message: "hi".to_string(),
            })
            .is_none()
        );
    }
    /// A meter for a fixed launch, so a row's three launch dimensions are never in question.
    fn meter(dir: &tempfile::TempDir) -> UsageMeter {
        UsageMeter {
            meter: Arc::new(crate::store::usage::Usage::open(dir.path()).unwrap()),
            project: "01J0PROJECT".to_string(),
            harness: "claude-code".to_string(),
            account: "mdn".to_string(),
        }
    }

    fn report(model: &str, subagent: Option<&str>, spend: TokenSpend) -> UsageRecord {
        UsageRecord {
            // Repeated unchanged by every report of the turn, subagents included: it is a level,
            // and nothing here may accumulate it.
            used: 218_336,
            size: 1_000_000,
            cost_usd: None,
            model: Some(model.to_string()),
            spend: Some(spend),
            subagent: subagent.map(str::to_string),
        }
    }

    /// **Occupancy is a level; only a flow is recorded.** A report that moves the context ring and
    /// bills nothing must write no row at all — a zero row would claim the harness said "nothing
    /// spent", which is not the same thing as it having said nothing.
    #[test]
    fn a_report_without_spend_writes_nothing() {
        let dir = tempfile::TempDir::new().unwrap();
        let meter = meter(&dir);
        let record = UsageRecord {
            used: 30_984,
            size: 1_000_000,
            cost_usd: Some(0.053_007_8),
            model: Some("claude-sonnet-5".to_string()),
            spend: None,
            subagent: None,
        };

        assert!(usage_row(&meter, &record).is_none());
    }

    /// Turn 2 of `_data/ubiq-tape-1788688032.jsonl`, which spawned three subagents: Claude Code
    /// bills one report per model at turn end, and the subagents' work comes back stamped with
    /// the type that did it. The assertion is where the tokens land — parent and subagent in
    /// their own buckets, per model — because a meter that merges them is still a meter, just a
    /// lying one.
    #[test]
    fn a_turns_spend_splits_between_the_conversation_and_its_subagents() {
        let dir = tempfile::TempDir::new().unwrap();
        let meter = meter(&dir);
        let at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_788_687_865);

        let turn = [
            // What the turn billed on sonnet, the conversation's own.
            report(
                "claude-sonnet-5",
                None,
                TokenSpend {
                    input: 16,
                    output: 1_586,
                    thinking: 0,
                    cache_read: 175_250,
                    cache_creation: 43_070,
                },
            ),
            // And on haiku, which is what its subagents ran.
            report(
                "claude-haiku-5",
                Some("general-purpose"),
                TokenSpend {
                    input: 901,
                    output: 14,
                    thinking: 0,
                    cache_read: 0,
                    cache_creation: 0,
                },
            ),
            // A second subagent of the same type sums into the first's bucket.
            report(
                "claude-haiku-5",
                Some("general-purpose"),
                TokenSpend {
                    input: 100,
                    output: 1,
                    thinking: 0,
                    cache_read: 0,
                    cache_creation: 0,
                },
            ),
            // A third, of another type, does not.
            report(
                "claude-haiku-5",
                Some("explore"),
                TokenSpend {
                    input: 7,
                    output: 2,
                    thinking: 0,
                    cache_read: 0,
                    cache_creation: 0,
                },
            ),
        ];
        for record in &turn {
            meter
                .meter
                .record(at, &usage_row(&meter, record).unwrap())
                .unwrap();
        }

        let mut rows = meter.meter.history(0).unwrap();
        rows.sort_by(|a, b| (&a.model, &a.subagent).cmp(&(&b.model, &b.subagent)));
        assert_eq!(rows.len(), 3);

        let explore = &rows[0];
        assert_eq!(
            (explore.model.as_str(), explore.subagent.as_str()),
            ("claude-haiku-5", "explore")
        );
        assert_eq!(explore.tokens_in, 7);

        let general = &rows[1];
        assert_eq!(
            (general.model.as_str(), general.subagent.as_str()),
            ("claude-haiku-5", "general-purpose")
        );
        assert_eq!((general.tokens_in, general.tokens_out), (1_001, 15));

        let parent = &rows[2];
        assert_eq!(
            (parent.model.as_str(), parent.subagent.as_str()),
            ("claude-sonnet-5", "")
        );
        assert_eq!(parent.tokens_in, 16);
        assert_eq!(parent.tokens_out, 1_586);
        assert_eq!(parent.tokens_think, 0);
        // Cache read and cache creation are distinct to the harness; `tokens_other` is where the
        // meter stops distinguishing them.
        assert_eq!(parent.tokens_other, 175_250 + 43_070);

        // Every row carries the launch's dimensions, and none carries occupancy.
        for row in &rows {
            assert_eq!(row.project, "01J0PROJECT");
            assert_eq!(row.harness, "claude-code");
            assert_eq!(row.account, "mdn");
            assert_eq!((row.msgs_in, row.msgs_out, row.tool_calls), (0, 0, 0));
        }
        // 218_336 is the level every report above repeated. It is nowhere in the meter.
        assert!(rows.iter().all(|row| row.tokens_total() != 218_336));
    }

    /// A sink that keeps what it was sent, so a test can read the order things went in.
    struct Recorder {
        seen: Arc<Mutex<Vec<AgentInput>>>,
    }

    impl AgentInputSink for Recorder {
        fn send(&self, input: AgentInput) -> anyhow::Result<()> {
            self.seen.lock().unwrap().push(input);
            Ok(())
        }
    }

    /// A conversation with a recording sink and no pump — enough for `cancel`, which touches
    /// neither.
    fn recording() -> (Conversation, Arc<Mutex<Vec<AgentInput>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let conversation = Conversation {
            id: AgentId::generate(),
            input: Some(Arc::new(Recorder { seen: seen.clone() })),
            pump: None,
            kill: None,
            ended: Arc::new(AtomicBool::new(false)),
            seq: Arc::new(AtomicU64::new(0)),
            quiet: Arc::new(AtomicBool::new(false)),
            outstanding: Arc::new(Mutex::new(Vec::new())),
            session: Arc::new(Mutex::new(None)),
            first_reply: Arc::new(Mutex::new(None)),
        };
        (conversation, seen)
    }

    /// Upstream makes this the cancelling client's obligation, and it is not a courtesy: a
    /// harness holding an unanswered request has no timeout to fall back on, so the turn it
    /// belongs to would never end.
    #[test]
    fn a_cancel_answers_every_outstanding_permission_first() {
        let (conversation, seen) = recording();
        *conversation.outstanding.lock().unwrap() = vec!["r1".to_string(), "r2".to_string()];

        conversation.cancel().unwrap();

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3, "two answers, then the cancel itself");
        for (ix, request_id) in ["r1", "r2"].iter().enumerate() {
            let AgentInput::AnswerPermission {
                request_id: answered,
                outcome,
                updated_input,
            } = &seen[ix]
            else {
                panic!("expected an answer, got {:?}", seen[ix]);
            };
            assert_eq!(
                answered.as_str(),
                *request_id,
                "answered in the order they arrived"
            );
            assert_eq!(*outcome, PermissionOutcome::Cancelled);
            assert!(
                updated_input.is_none(),
                "the response carries an option id and nothing else"
            );
        }
        assert!(matches!(seen[2], AgentInput::Cancel));
    }

    /// Answered once, and then not again: a second cancel has nothing left to say for it.
    #[test]
    fn an_answered_permission_is_not_cancelled_afterwards() {
        let (conversation, seen) = recording();
        *conversation.outstanding.lock().unwrap() = vec!["r1".to_string()];

        conversation
            .answer_permission("r1".to_string(), "allow_once".to_string())
            .unwrap();
        conversation.cancel().unwrap();

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "the answer, then the bare cancel");
        assert!(matches!(
            &seen[0],
            AgentInput::AnswerPermission {
                outcome: PermissionOutcome::Selected { .. },
                ..
            }
        ));
        assert!(matches!(seen[1], AgentInput::Cancel));
    }

    /// A cancel keeps the harness — so a stop cannot be one. It answers what is outstanding for
    /// the same reason a cancel does, then sends the teardown that actually closes the child's
    /// input; reusing `cancel()` here would leave the process running.
    #[test]
    fn a_stop_tears_down_rather_than_cancelling() {
        let (conversation, seen) = recording();
        *conversation.outstanding.lock().unwrap() = vec!["r1".to_string()];

        conversation.stop(false);

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "the answer, then the teardown");
        assert!(matches!(
            &seen[0],
            AgentInput::AnswerPermission {
                outcome: PermissionOutcome::Cancelled,
                ..
            }
        ));
        assert!(
            matches!(seen[1], AgentInput::Shutdown),
            "a stop sends Shutdown, not Cancel: {:?}",
            seen[1]
        );
    }
}
