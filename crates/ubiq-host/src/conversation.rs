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

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;

use agent_manager::io::{
    AgentEvent, AgentInput, AgentInputSink, Content, IoBridge, PermissionOutcome,
};
use ubiq_proto::bus::Mailbox;
use ubiq_proto::conversation::{
    ConfigCategory, ConfigChoice, ConfigOption, ConfigValue, ConvContent, ConvUpdate,
    PermissionKind, PermissionOption, PlanEntry, PlanPriority, PlanStatus, RateLimitRecord,
    StopReason, ToolCallPatch, ToolCallRecord, ToolContent, ToolKind, ToolLocation, ToolStatus,
    UsageRecord,
};
use ubiq_proto::messages::Message;
use ubiq_proto::work::AgentId;

/// A running conversation, as the coordinator holds it.
///
/// It owns no bridge: the pump thread does. What is left here is the way in —
/// and the way to stop, which is the same thing, because closing a harness's
/// input is what makes it exit.
pub struct Conversation {
    id: AgentId,
    input: Option<Arc<dyn AgentInputSink>>,
    pump: Option<thread::JoinHandle<()>>,
    /// Set by the pump, just before it returns, whether the harness ended on its own or the
    /// window went first. The coordinator polls this — the same shape it already polls
    /// `active_searches`'s own "is this over" flag — to reap a conversation whose harness quit
    /// without anybody asking it to.
    ended: Arc<AtomicBool>,
    /// The pump's own sequence counter, published after every send, so an unload can hand the
    /// coordinator the last `seq` it reached without racing the pump for it.
    seq: Arc<AtomicU64>,
    /// Set by `stop(true)` before the pump is asked to exit. Read by the pump on its way out to
    /// decide whether to send the final `ConversationEnded` — an unload wants exactly one
    /// lifecycle message (`ConversationUnloaded`), not that plus this.
    quiet: Arc<AtomicBool>,
}

impl Conversation {
    /// Start pumping `bridge` onto `out`, stamping every message with `id`.
    ///
    /// `start_seq` is where the sequence counter picks up rather than always zero, so a
    /// conversation that said something before this harness existed — P3's pending picker, over
    /// `ConversationUpdate` — and the harness's own first frame are one unbroken sequence.
    pub fn start(id: AgentId, bridge: Box<dyn IoBridge>, out: Mailbox, start_seq: u64) -> Self {
        let input = bridge.input();
        let ended = Arc::new(AtomicBool::new(false));
        let seq = Arc::new(AtomicU64::new(start_seq));
        let quiet = Arc::new(AtomicBool::new(false));
        let pump_ended = ended.clone();
        let pump_seq = seq.clone();
        let pump_quiet = quiet.clone();
        let pump = thread::Builder::new()
            .name(format!("agent-{id}"))
            .spawn(move || pump(id, bridge, out, start_seq, pump_ended, pump_seq, pump_quiet))
            .ok();

        Self {
            id,
            input,
            pump,
            ended,
            seq,
            quiet,
        }
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

    /// Interrupt the turn in flight.
    pub fn cancel(&self) -> anyhow::Result<()> {
        self.send(AgentInput::Cancel)
    }

    /// Answer a permission request by naming one of the options it offered.
    pub fn answer_permission(&self, request_id: String, option_id: String) -> anyhow::Result<()> {
        self.send(AgentInput::AnswerPermission {
            request_id,
            outcome: PermissionOutcome::Selected { option_id },
            updated_input: None,
        })
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
    /// Cancelling closes the child's input, which is what makes it exit; the
    /// bridge's own teardown then gives it a bounded window to drain before
    /// killing it. So the wait here is for a thread that is already ending
    /// rather than one that has to be interrupted.
    ///
    /// `quiet` set is an unload: the pump skips its final `ConversationEnded` so the coordinator's
    /// own `ConversationUnloaded` is the only lifecycle message this stop produces.
    pub fn stop(mut self, quiet: bool) -> u64 {
        self.quiet.store(quiet, Ordering::Relaxed);
        let _ = self.cancel();
        if let Some(pump) = self.pump.take() {
            let _ = pump.join();
        }
        tracing::debug!(agent = %self.id, "conversation stopped");
        self.seq.load(Ordering::Relaxed)
    }
}

/// The pump thread: read the bridge until it ends, and put everything it says
/// on the bus.
fn pump(
    id: AgentId,
    mut bridge: Box<dyn IoBridge>,
    out: Mailbox,
    start_seq: u64,
    ended: Arc<AtomicBool>,
    seq_counter: Arc<AtomicU64>,
    quiet: Arc<AtomicBool>,
) {
    let mut seq = start_seq;
    let mut stop_reason = StopReason::EndTurn;

    loop {
        let event = match bridge.next_event() {
            Ok(Some(event)) => event,
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
        }

        let Some(update) = map_event(event) else {
            continue;
        };

        seq += 1;
        seq_counter.store(seq, Ordering::Relaxed);
        tracing::debug!(agent = %id, seq, update = ?update, "conversation update");
        if !out.send(Message::ConversationUpdate {
            agent_id: id,
            seq,
            update: Box::new(update),
        }) {
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
        } => ConvUpdate::AgentChunk {
            content: map_content(content),
            message_id,
        },
        AgentEvent::AgentThoughtChunk {
            content,
            message_id,
        } => ConvUpdate::ThoughtChunk {
            content: map_content(content),
            message_id,
        },

        AgentEvent::ToolCall { call } => ConvUpdate::ToolCall(ToolCallRecord {
            id: call.id,
            title: call.title,
            kind: map_kind(call.kind),
            status: map_status(call.status),
            content: call.content.into_iter().map(map_tool_content).collect(),
            locations: call.locations.into_iter().map(map_location).collect(),
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
        } => ConvUpdate::Usage(UsageRecord {
            used,
            size,
            cost_usd: cost.map(|cost| cost.amount),
            model,
        }),

        AgentEvent::RateLimitUpdate {
            five_hour,
            seven_day,
            status,
        } => ConvUpdate::RateLimit(RateLimitRecord {
            five_hour_pct: five_hour.as_ref().map(|w| w.utilization_pct),
            five_hour_resets_at: five_hour.as_ref().map(|w| w.resets_at),
            seven_day_pct: seven_day.as_ref().map(|w| w.utilization_pct),
            seven_day_resets_at: seven_day.as_ref().map(|w| w.resets_at),
            status,
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

    /// Blocks in `next_event` until cancelled, the same way a real child blocks until closing its
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
            if matches!(input, AgentInput::Cancel) {
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
        })
        .unwrap();
        assert_eq!(
            update,
            ConvUpdate::AgentChunk {
                content: ConvContent::Text("hello".to_string()),
                message_id: Some("m1".to_string()),
            }
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
        })
        .unwrap();
        let ConvUpdate::Usage(usage) = update else {
            panic!("expected usage");
        };
        assert_eq!(usage.size, 200_000);
        assert_eq!(usage.cost_usd, Some(0.5));
        assert_eq!(usage.context_pct(), Some(0));
    }

    #[test]
    fn rate_limit_maps_both_windows_and_the_status() {
        let update = map_event(AgentEvent::RateLimitUpdate {
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
}
