//! One live agent's pump, over the bus, with a bridge that is not a process.
//!
//! `Conversation::start` takes any `IoBridge`, so the whole of the pump — the
//! mapping, the sequence numbering, the ending — is testable without a
//! harness, a pipe or a second of waiting. What a real harness contributes is
//! covered in `crates/agent-manager`'s own bridge tests; what is covered here
//! is everything between an event and the bus.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_manager::io::{
    AgentEvent, AgentInput, AgentInputSink, Content, IoBridge, StopReason as LibStop, ToolCall,
    ToolCallUpdate, ToolKind, ToolStatus,
};
use ubiq_host::conversation::Conversation;
use ubiq_proto::bus::{self, Client, HostEnd, To};
use ubiq_proto::conversation::{ConvContent, ConvUpdate, StopReason, ToolStatus as WireStatus};
use ubiq_proto::messages::Message;
use ubiq_proto::work::AgentId;

/// Long enough for a thread to be scheduled on a loaded machine.
const PATIENCE: Duration = Duration::from_secs(5);

/// A bridge that says what it was told to say and then ends. Its input side
/// records what was sent to it, so a prompt can be followed to the far end.
struct Scripted {
    events: mpsc::Receiver<AgentEvent>,
    sent: Arc<Mutex<Vec<AgentInput>>>,
}

struct ScriptedInput {
    sent: Arc<Mutex<Vec<AgentInput>>>,
}

impl AgentInputSink for ScriptedInput {
    fn send(&self, input: AgentInput) -> anyhow::Result<()> {
        self.sent.lock().unwrap().push(input);
        Ok(())
    }
}

impl Scripted {
    /// A bridge that will emit `events` and then reach end of stream.
    fn new(events: Vec<AgentEvent>) -> (Self, Arc<Mutex<Vec<AgentInput>>>) {
        let (tx, rx) = mpsc::channel();
        for event in events {
            tx.send(event).unwrap();
        }
        let sent = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                events: rx,
                sent: Arc::clone(&sent),
            },
            sent,
        )
    }
}

impl IoBridge for Scripted {
    fn send(&mut self, input: AgentInput) -> anyhow::Result<()> {
        self.sent.lock().unwrap().push(input);
        Ok(())
    }

    fn next_event(&mut self) -> anyhow::Result<Option<AgentEvent>> {
        Ok(self.events.recv().ok())
    }

    fn input(&self) -> Option<Arc<dyn AgentInputSink>> {
        Some(Arc::new(ScriptedInput {
            sent: Arc::clone(&self.sent),
        }))
    }
}

/// A bridge with no detached input handle — what a one-shot harness is.
struct OneShot;

impl IoBridge for OneShot {
    fn send(&mut self, _input: AgentInput) -> anyhow::Result<()> {
        Ok(())
    }

    fn next_event(&mut self) -> anyhow::Result<Option<AgentEvent>> {
        Ok(None)
    }
}

/// The host end comes back with the client because a mailbox is taken from it,
/// and the hub because dropping it would detach the client under the test.
fn bus_pair() -> (bus::Hub, HostEnd, Client) {
    let (hub, host) = bus::hub();
    let client = hub.connect();
    (hub, host, client)
}

fn drain(client: &Client, count: usize) -> Vec<Message> {
    let mut got = Vec::new();
    while got.len() < count {
        match client.from_host().recv_timeout(PATIENCE) {
            Ok(message) => got.push(message),
            Err(_) => panic!("only {} of {count} messages arrived: {got:#?}", got.len()),
        }
    }
    got
}

fn text(text: &str) -> AgentEvent {
    AgentEvent::AgentMessageChunk {
        content: Content::text(text),
        message_id: Some("m1".to_string()),
    }
}

/// The whole path: an event goes in, a bus message comes out, stamped with the
/// agent that produced it and a sequence number that starts at one.
#[test]
fn what_the_harness_says_reaches_the_bus_in_order() {
    let (_hub, host_end, client) = bus_pair();
    let id = AgentId::generate();
    let (bridge, _) = Scripted::new(vec![
        AgentEvent::SessionStarted {
            session_id: Some("s1".to_string()),
            model: Some("claude-opus-5".to_string()),
            mode: None,
            tools: Vec::new(),
            agents: Vec::new(),
        },
        text("hello"),
        AgentEvent::TurnEnded {
            stop_reason: LibStop::EndTurn,
            error: None,
        },
    ]);

    let host = host_end.mailbox(To::Client(client.id()));
    let conversation = Conversation::start(id, Box::new(bridge), host);
    let messages = drain(&client, 4);

    let seqs: Vec<u64> = messages
        .iter()
        .filter_map(|message| match message {
            Message::ConversationUpdate { seq, .. } => Some(*seq),
            _ => None,
        })
        .collect();
    assert_eq!(seqs, vec![1, 2, 3], "one per event, in order");

    for message in &messages[..3] {
        let Message::ConversationUpdate { agent_id, .. } = message else {
            panic!("expected an update, got {message:?}");
        };
        assert_eq!(*agent_id, id, "every update names the agent it came from");
    }

    let Message::ConversationUpdate { update, .. } = &messages[1] else {
        panic!("expected an update");
    };
    assert_eq!(
        **update,
        ConvUpdate::AgentChunk {
            content: ConvContent::Text("hello".to_string()),
            message_id: Some("m1".to_string()),
        }
    );

    // End of stream is the harness going, and the pump says so itself.
    assert!(matches!(
        messages[3],
        Message::ConversationEnded {
            stop_reason: StopReason::EndTurn,
            ..
        }
    ));
    conversation.stop();
}

/// A tool call and its completion are two messages about one block, which is
/// what lets a transcript patch rather than redraw.
#[test]
fn a_tool_call_and_its_completion_keep_the_same_id() {
    let (_hub, host_end, client) = bus_pair();
    let mut call = ToolCall::new("t1", "Bash ls");
    call.kind = ToolKind::Execute;
    call.status = ToolStatus::InProgress;
    let (bridge, _) = Scripted::new(vec![
        AgentEvent::ToolCall { call },
        AgentEvent::ToolCallUpdate {
            update: ToolCallUpdate::finished("t1", ToolStatus::Completed),
        },
    ]);

    let host = host_end.mailbox(To::Client(client.id()));
    let conversation = Conversation::start(AgentId::generate(), Box::new(bridge), host);
    let messages = drain(&client, 3);

    let Message::ConversationUpdate { update, .. } = &messages[0] else {
        panic!("expected an update");
    };
    let ConvUpdate::ToolCall(record) = &**update else {
        panic!("expected a tool call, got {update:?}");
    };
    assert_eq!(record.id, "t1");

    let Message::ConversationUpdate { update, .. } = &messages[1] else {
        panic!("expected an update");
    };
    let ConvUpdate::ToolCallUpdate(patch) = &**update else {
        panic!("expected a patch, got {update:?}");
    };
    assert_eq!(patch.id, "t1", "the patch names the call it completes");
    assert_eq!(patch.status, Some(WireStatus::Completed));
    assert_eq!(patch.title, None, "a patch changes only what it names");
    conversation.stop();
}

/// A prompt reaches the harness from a thread that does not own the bridge —
/// which is the whole reason `IoBridge::input` exists, since `next_event`
/// blocks and the pump is sitting in it.
#[test]
fn a_prompt_reaches_a_bridge_the_pump_thread_owns() {
    let (_hub, host_end, client) = bus_pair();
    let (bridge, sent) = Scripted::new(Vec::new());

    let host = host_end.mailbox(To::Client(client.id()));
    let conversation = Conversation::start(AgentId::generate(), Box::new(bridge), host);
    assert!(conversation.accepts_input());

    conversation.prompt("do the thing".to_string()).unwrap();

    let sent = sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].prompt_text().as_deref(), Some("do the thing"));
}

/// A harness that takes nothing after launch refuses here rather than
/// swallowing the turn, so a composer that should not have offered to send
/// finds out.
#[test]
fn a_one_shot_harness_refuses_a_second_turn() {
    let (_hub, host_end, client) = bus_pair();
    let host = host_end.mailbox(To::Client(client.id()));
    let conversation = Conversation::start(AgentId::generate(), Box::new(OneShot), host);

    assert!(!conversation.accepts_input());
    assert!(conversation.prompt("again".to_string()).is_err());
}

/// The multiplexing claim, tested rather than asserted: two agents streaming
/// at once down one bus, told apart by nothing but the id on each message.
#[test]
fn two_conversations_share_one_bus_without_interleaving() {
    let (_hub, host_end, client) = bus_pair();
    let first = AgentId::generate();
    let second = AgentId::generate();

    let (one, _) = Scripted::new(vec![text("from one"), text("still one")]);
    let (two, _) = Scripted::new(vec![text("from two"), text("still two")]);

    let a = Conversation::start(
        first,
        Box::new(one),
        host_end.mailbox(To::Client(client.id())),
    );
    let b = Conversation::start(
        second,
        Box::new(two),
        host_end.mailbox(To::Client(client.id())),
    );

    let messages = drain(&client, 6);

    // Order is promised per agent, not across them, so each agent's own
    // sequence is what has to be intact.
    for id in [first, second] {
        let seqs: Vec<u64> = messages
            .iter()
            .filter_map(|message| match message {
                Message::ConversationUpdate { agent_id, seq, .. } if *agent_id == id => Some(*seq),
                _ => None,
            })
            .collect();
        assert_eq!(seqs, vec![1, 2], "agent {id} numbered its own updates");
    }

    a.stop();
    b.stop();
}
