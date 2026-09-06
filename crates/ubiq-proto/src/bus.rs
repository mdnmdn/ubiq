//! The bus: the switchboard between the one host and the windows attached to it, and the byte
//! streams a pane is handed.
//!
//! Every half lives in one process today, so the bus is a set of unbounded channels carrying
//! [`Message`] values. Unbounded is the contract, not an accident: a window that falls behind must
//! never stall the host's reader, because that stalls the harness.
//!
//! **One host, many clients.** A window attaches with [`Hub::connect`] and gets a [`Client`]; the
//! host reads every client through one [`HostEnd`] and answers each message to somebody — the one
//! window that owns a pane, or every window at once. Attaching and detaching are facts about the
//! transport rather than things either half says, so they are [`FromClient`] variants and not
//! messages.
//!
//! The emulator wants a `Read` and a `Write`. It gets [`PaneOutput`] and [`PaneInput`], which are
//! bus endpoints for one pane ID — never a pseudo-terminal. That is what keeps the UI honest about
//! the pane being an ID plus a byte stream.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use parking_lot::Mutex;

use crate::ids::PaneId;
use crate::messages::Message;

/// One attached window, for as long as it is attached.
///
/// Not one of the contract's ids: it never serialises and never persists. A detached host would
/// take it from the connection it accepted rather than from anything a client said.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ClientId(u64);

impl std::fmt::Display for ClientId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "client {}", self.0)
    }
}

/// Who a host → interface message is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum To {
    /// The one window that owns the pane, or that asked the question.
    Client(ClientId),
    /// Every attached window. The project family goes this way, so every picker agrees by
    /// construction rather than by each window asking again.
    Everyone,
}

/// What the host reads. Two of the three are not contract messages, because a window attaching or
/// going away is not something it says.
///
/// `Said` is as wide as the contract's widest variant, on purpose: [`Message`] is already the one
/// place a size trade-off is made, for the terminal chunks on the hot path — boxing it again here
/// would just move the cost to every dispatch site for a clippy heuristic, not for a real one.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum FromClient {
    Connected(ClientId),
    Said {
        client: ClientId,
        message: Message,
    },
    /// The window has gone. Whatever it owned is the host's to reap.
    Gone(ClientId),
}

/// The routing table: every attached client's inbox.
type Clients = Arc<Mutex<HashMap<ClientId, flume::Sender<Message>>>>;

/// The switchboard. Cloneable and process-wide: the binary starts the host with the other end and
/// hands this to the interface, which mints one client per window.
#[derive(Clone)]
pub struct Hub {
    to_host: flume::Sender<FromClient>,
    clients: Clients,
    next: Arc<AtomicU64>,
}

/// The host's end: one inbox for every client, and the routing table to answer through.
pub struct HostEnd {
    inbox: flume::Receiver<FromClient>,
    clients: Clients,
}

/// Open the bus. One [`HostEnd`] for the process, and a [`Hub`] that mints a client per window.
pub fn hub() -> (Hub, HostEnd) {
    let (to_host, inbox) = flume::unbounded();
    let clients: Clients = Arc::new(Mutex::new(HashMap::new()));
    (
        Hub {
            to_host,
            clients: clients.clone(),
            next: Arc::new(AtomicU64::new(0)),
        },
        HostEnd { inbox, clients },
    )
}

impl Hub {
    /// Attach a window. The host is told now, and told again when the client is dropped.
    pub fn connect(&self) -> Client {
        let id = ClientId(self.next.fetch_add(1, Ordering::Relaxed));
        let (to_client, from_host) = flume::unbounded();
        self.clients.lock().insert(id, to_client);
        let _ = self.to_host.send(FromClient::Connected(id));
        Client {
            id,
            to_host: self.to_host.clone(),
            from_host,
            clients: Some(self.clients.clone()),
        }
    }
}

impl HostEnd {
    /// The host's run loop reads here. Blocks; ends when the hub and every client have gone.
    pub fn recv(&self) -> Result<FromClient, flume::RecvError> {
        self.inbox.recv()
    }

    /// The same, giving up after `wait`. The host uses this when it has work of its own due — a
    /// debounced preference that has to be written whether or not anybody says anything else.
    pub fn recv_timeout(
        &self,
        wait: std::time::Duration,
    ) -> Result<FromClient, flume::RecvTimeoutError> {
        self.inbox.recv_timeout(wait)
    }

    /// A sink that already knows who it is talking to, for a thread that must not have to learn
    /// the routing table — a pane's reader, and its reaper.
    pub fn mailbox(&self, to: To) -> Mailbox {
        match to {
            // Resolved once, here, so the hot path never takes the lock. A departed client's
            // sender fails, which is exactly what stops the reader thread.
            To::Client(id) => Mailbox(match self.clients.lock().get(&id) {
                Some(sender) => Sink::One(sender.clone()),
                None => Sink::Gone,
            }),
            To::Everyone => Mailbox(Sink::All(self.clients.clone())),
        }
    }

    /// Address one message. Never blocks, and a client that has gone is not an error the host can
    /// act on.
    pub fn send(&self, to: To, message: Message) {
        self.mailbox(to).send(message);
    }

    /// Every client currently attached.
    pub fn attached(&self) -> Vec<ClientId> {
        let mut ids: Vec<ClientId> = self.clients.lock().keys().copied().collect();
        ids.sort();
        ids
    }
}

/// A pre-addressed host-side sink.
#[derive(Clone)]
pub struct Mailbox(Sink);

#[derive(Clone)]
enum Sink {
    One(flume::Sender<Message>),
    All(Clients),
    /// The client had already gone when the mailbox was made.
    Gone,
}

impl Mailbox {
    /// Post a message, and answer whether the destination is still reachable.
    ///
    /// The answer is what a pane's reader thread stops on: once the window that owned the pane has
    /// gone, nothing is left to draw its output, and a reader that kept draining the
    /// pseudo-terminal into nowhere would keep the harness alive with it.
    pub fn send(&self, message: Message) -> bool {
        tape().record(Direction::Inbound, &message);
        match &self.0 {
            Sink::One(sender) => sender.send(message).is_ok(),
            // A broadcast with nobody attached is not a reason for anything to stop.
            Sink::All(clients) => {
                for sender in clients.lock().values() {
                    let _ = sender.send(message.clone());
                }
                true
            }
            Sink::Gone => false,
        }
    }
}

/// A window's end of the bus.
///
/// `clients` is `None` for a [`detached`] client: it was never inserted into a `Hub`'s routing
/// table, so it has nothing to remove itself from on drop. A `Hub`-backed client always carries
/// `Some`.
pub struct Client {
    id: ClientId,
    to_host: flume::Sender<FromClient>,
    from_host: flume::Receiver<Message>,
    clients: Option<Clients>,
}

/// A process-wide counter for detached client ids, kept apart from any `Hub`'s own so the two
/// numberings can never collide into the same id meaning two different clients.
fn next_detached_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Build a [`Client`] with no [`Hub`] behind it, for a socket pump to drive instead of an
/// in-process host. Returns the client and the [`Detached`] handle a pump needs: the outbound
/// side to read what the client said and write to the socket, and the inbound side to feed what
/// arrived from the socket back to the client.
///
/// This mints the `Client` half of the contract only — no networking, no framing, nothing that
/// touches a socket. That is the transport layered on top, in the host crate, in a later phase.
pub fn detached() -> (Client, Detached) {
    let id = ClientId(next_detached_id());
    let (to_host, said) = flume::unbounded();
    let (to_client, from_host) = flume::unbounded();
    (
        Client {
            id,
            to_host,
            from_host,
            clients: None,
        },
        Detached {
            said,
            deliver: to_client,
        },
    )
}

/// The two halves of a [`detached`] client that a network pump needs, on the far side from the
/// `Client` itself.
pub struct Detached {
    said: flume::Receiver<FromClient>,
    deliver: flume::Sender<Message>,
}

impl Detached {
    /// What the client said — [`Client::send`] and the callbacks in [`Outbox`] and [`PaneInput`]
    /// all arrive here as [`FromClient::Said`]. A pump encodes each and writes it to the socket.
    pub fn said(&self) -> &flume::Receiver<FromClient> {
        &self.said
    }

    /// Hand the client a message that arrived from the socket. It surfaces on
    /// [`Client::from_host`], exactly as if a `Hub` had routed it.
    pub fn deliver(&self) -> &flume::Sender<Message> {
        &self.deliver
    }
}

impl Client {
    pub fn id(&self) -> ClientId {
        self.id
    }

    /// Say something to the host. A closed bus is not an error the UI can act on, so it is dropped
    /// rather than surfaced.
    pub fn send(&self, message: Message) {
        tape().record(Direction::Outbound, &message);
        let _ = self.to_host.send(FromClient::Said {
            client: self.id,
            message,
        });
    }

    /// What the host has said to this window, drained by the UI's router task.
    pub fn from_host(&self) -> &flume::Receiver<Message> {
        &self.from_host
    }

    /// A sender for the callbacks the emulator invokes on its own — a resize it measured, say —
    /// which need to reach the host without a window in hand.
    pub fn sender(&self) -> Outbox {
        Outbox {
            client: self.id,
            to_host: self.to_host.clone(),
        }
    }

    /// The write half for one pane: keystrokes leave as [`Message::TerminalInput`].
    pub fn input(&self, pane_id: PaneId) -> PaneInput {
        PaneInput {
            pane_id,
            out: self.sender(),
        }
    }
}

/// A window has gone, and its connection goes with it. The host is told, so it can reap the panes
/// that window owned — nothing else drops now that the host outlives every window.
///
/// A detached client has no routing table to remove itself from — `clients` is `None` — but the
/// `Gone` announcement still goes out, because whatever is driving the socket on the other end
/// still needs to know.
impl Drop for Client {
    fn drop(&mut self) {
        if let Some(clients) = &self.clients {
            clients.lock().remove(&self.id);
        }
        let _ = self.to_host.send(FromClient::Gone(self.id));
    }
}

/// A cloneable way to speak to the host with no window in hand. It carries the client id, so a
/// callback is still attributed to the window it came from.
#[derive(Clone)]
pub struct Outbox {
    client: ClientId,
    to_host: flume::Sender<FromClient>,
}

impl Outbox {
    pub fn send(&self, message: Message) {
        let _ = self.to_host.send(FromClient::Said {
            client: self.client,
            message,
        });
    }
}

/// The write half handed to a pane's emulator.
pub struct PaneInput {
    pane_id: PaneId,
    out: Outbox,
}

impl Write for PaneInput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.out
            .to_host
            .send(FromClient::Said {
                client: self.out.client,
                message: Message::TerminalInput {
                    pane_id: self.pane_id,
                    bytes: buf.to_vec(),
                },
            })
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "bus closed"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The read half handed to a pane's emulator: the output the router routed to this pane.
///
/// Reads block, because the emulator reads on a thread of its own. Dropping the matching sender is
/// how a pane is told its harness is done — the read returns end of stream.
pub struct PaneOutput {
    chunks: flume::Receiver<Vec<u8>>,
    /// What is left of the chunk a previous read could not finish.
    pending: Vec<u8>,
    at: usize,
}

/// Open a pane's output stream. The sender goes to the router, the reader to the emulator.
pub fn pane_output() -> (flume::Sender<Vec<u8>>, PaneOutput) {
    let (tx, chunks) = flume::unbounded();
    (
        tx,
        PaneOutput {
            chunks,
            pending: Vec::new(),
            at: 0,
        },
    )
}

impl Read for PaneOutput {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        // An empty chunk must not read as end of stream, so wait for one with bytes in it.
        while self.at >= self.pending.len() {
            match self.chunks.recv() {
                Ok(chunk) => {
                    self.pending = chunk;
                    self.at = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.pending.len() - self.at);
        buf[..n].copy_from_slice(&self.pending[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

// ── The tape ────────────────────────────────────────────────────────

// A process-wide ring of everything that crossed the bus, for the debug viewer.
//
// Modelled on `crate::log::Logs`, and for the same reason: entries travel one way. The bus writes
// and never reads; the window reads and never writes anything the bus can see. An entry is a
// message already serialised, so nothing here holds a pane's state, a path or a descriptor. The
// pane hot path is never taped — see `Tape::record`.

/// Which way a taped message was going.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// Host → window.
    Inbound,
    /// Window → host.
    Outbound,
}

impl Direction {
    pub fn label(self) -> &'static str {
        match self {
            Direction::Inbound => "IN",
            Direction::Outbound => "OUT",
        }
    }
}

/// One message, as it crossed.
#[derive(Clone, Debug)]
pub struct TapeEntry {
    /// Monotonic across the process, so a row has an identity the ring cannot reuse.
    pub seq: u64,
    pub at: SystemTime,
    pub direction: Direction,
    /// The message's own variant name, for the row's label. It is what travels in `type`.
    pub kind: String,
    /// The message as JSON, truncated at [`TAPE_MAX_JSON`].
    pub json: String,
    /// The harness's own line behind it, where the message carried one.
    pub raw: Option<String>,
    /// Which agent it was about, where the payload names one. Lifted out of the message so a row
    /// — and a line in a dump — can be read by agent without parsing the body first.
    pub agent: Option<String>,
}

/// How many entries the ring keeps. Small on purpose: this is a debug viewer's window onto the
/// last few seconds of traffic, not an audit log.
pub const TAPE_CAPACITY: usize = 500;

/// How much of one message's JSON is kept. A settings write or a search batch is far longer than
/// anything a row can show.
pub const TAPE_MAX_JSON: usize = 8 * 1024;

/// The environment variable that turns continuous capture on, naming the folder to write into.
/// Unset — the normal case — and nothing is ever written: the tape is a ring in memory.
pub const TAPE_DIR_ENV: &str = "UBIQ_TAPE_DIR";

/// The ring the bus writes and the debug viewer reads.
pub struct Tape {
    inner: Mutex<TapeInner>,
}

struct TapeInner {
    entries: VecDeque<Arc<TapeEntry>>,
    next_seq: u64,
    /// How many entries the ring has dropped off its front since the last clear.
    dropped: u64,
    /// One per viewer. The message is a nudge, not an entry — the reader takes what it wants from
    /// the ring, so a listener that misses a nudge misses nothing.
    listeners: Vec<flume::Sender<()>>,
    /// The capture file, opened on the first message when [`TAPE_DIR_ENV`] named a folder. `None`
    /// once opening has failed, so a bad folder costs one attempt rather than one per message.
    capture: Option<std::fs::File>,
    capture_tried: bool,
}

impl Tape {
    fn new() -> Self {
        Self {
            inner: Mutex::new(TapeInner {
                entries: VecDeque::with_capacity(64),
                next_seq: 0,
                dropped: 0,
                listeners: Vec::new(),
                capture: None,
                capture_tried: false,
            }),
        }
    }

    /// Tape one message, dropping the oldest if the ring is full.
    ///
    /// The pane family is skipped entirely: a tape that serialised every terminal chunk would put
    /// a JSON encoder on the path between the pseudo-terminal and the screen, and stall the
    /// harness — the one thing the bus exists to avoid. A message that will not serialise is
    /// dropped rather than recorded as an error nobody can act on.
    fn record(&self, direction: Direction, message: &Message) {
        if matches!(
            message,
            Message::TerminalOutput { .. }
                | Message::TerminalInput { .. }
                | Message::TerminalResize { .. }
        ) {
            return;
        }

        let Ok(value) = serde_json::to_value(message) else {
            return;
        };
        let kind = value
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Unknown")
            .to_string();
        let agent = value
            .get("payload")
            .and_then(|payload| payload.get("agent_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let full = value.to_string();
        let mut json = full.clone();
        if json.len() > TAPE_MAX_JSON {
            let mut cut = TAPE_MAX_JSON;
            while !json.is_char_boundary(cut) {
                cut -= 1;
            }
            json.truncate(cut);
            json.push('…');
        }
        let raw = match message {
            Message::ConversationUpdate { raw, .. } => raw.clone(),
            _ => None,
        };

        let mut inner = self.inner.lock();
        let seq = inner.next_seq;
        inner.next_seq += 1;
        let at = SystemTime::now();

        // Continuous capture, when the environment asked for it. The file gets the message
        // untruncated — the 8KB cut is what a row can show, not what an analysis can use.
        if let Some(file) = inner.capture() {
            let line = dump_line(
                seq,
                at,
                direction,
                &kind,
                agent.as_deref(),
                &full,
                raw.as_deref(),
            );
            let _ = std::io::Write::write_all(file, line.as_bytes());
        }

        inner.entries.push_back(Arc::new(TapeEntry {
            seq,
            at,
            direction,
            kind,
            json,
            raw,
            agent,
        }));
        while inner.entries.len() > TAPE_CAPACITY {
            inner.entries.pop_front();
            inner.dropped += 1;
        }
        inner.listeners.retain(|listener| listener.send(()).is_ok());
    }

    /// Everything the ring holds, oldest first. Entries are shared rather than copied, so a
    /// snapshot costs a pointer each and the ring is never held across a frame.
    pub fn snapshot(&self) -> Vec<Arc<TapeEntry>> {
        self.inner.lock().entries.iter().cloned().collect()
    }

    /// How many entries the ring holds, and how many it has dropped off its front.
    pub fn counts(&self) -> (usize, u64) {
        let inner = self.inner.lock();
        (inner.entries.len(), inner.dropped)
    }

    /// Forget everything. The viewers are woken, because what they are showing has just gone.
    pub fn clear(&self) {
        let mut inner = self.inner.lock();
        inner.entries.clear();
        inner.dropped = 0;
        inner.listeners.retain(|listener| listener.send(()).is_ok());
    }

    /// Ask to be told when a message is taped. Dropping the receiver is how a viewer unsubscribes.
    pub fn subscribe(&self) -> flume::Receiver<()> {
        let (sender, receiver) = flume::unbounded();
        self.inner.lock().listeners.push(sender);
        receiver
    }

    /// Write everything the ring holds to a file, and answer with the path.
    ///
    /// One JSON object per line — `seq`, `at_ms`, `dir`, `kind`, `agent`, the `message` itself as
    /// an object, and the harness's `raw` line where one travelled — so tokens, agent ids and
    /// anything else are `jq`'s to pull out without a parser of their own. What the ring holds is
    /// what is written, so a message longer than [`TAPE_MAX_JSON`] lands truncated, as a string;
    /// set [`TAPE_DIR_ENV`] instead for the untruncated stream from the first message on.
    pub fn dump(&self) -> std::io::Result<std::path::PathBuf> {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|since| since.as_secs())
            .unwrap_or_default();
        let path = tape_dir().join(format!("ubiq-tape-{stamp}.jsonl"));
        let entries = self.snapshot();
        let mut body = String::new();
        for entry in &entries {
            body.push_str(&dump_line(
                entry.seq,
                entry.at,
                entry.direction,
                &entry.kind,
                entry.agent.as_deref(),
                &entry.json,
                entry.raw.as_deref(),
            ));
        }
        std::fs::write(&path, body)?;
        Ok(path)
    }
}

impl TapeInner {
    /// The capture file, opened on demand. `None` is the normal case: nothing asked for one.
    fn capture(&mut self) -> Option<&mut std::fs::File> {
        if !self.capture_tried {
            self.capture_tried = true;
            if let Some(dir) = std::env::var_os(TAPE_DIR_ENV) {
                let dir = std::path::PathBuf::from(dir);
                let path = dir.join(format!("ubiq-tape-{}.jsonl", std::process::id()));
                let _ = std::fs::create_dir_all(&dir);
                self.capture = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .ok();
            }
        }
        self.capture.as_mut()
    }
}

/// Where a dump lands: the folder [`TAPE_DIR_ENV`] names, or the machine's temporary one.
fn tape_dir() -> std::path::PathBuf {
    std::env::var_os(TAPE_DIR_ENV)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// One line of a dump: the entry's own fields, with the message embedded as an object where it
/// still parses. A truncated body is written as a string instead, because half an object is not
/// one and a reader deserves to see which it got.
fn dump_line(
    seq: u64,
    at: SystemTime,
    direction: Direction,
    kind: &str,
    agent: Option<&str>,
    json: &str,
    raw: Option<&str>,
) -> String {
    let at_ms = at
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(json)
        .unwrap_or_else(|_| serde_json::Value::String(json.to_string()));
    let line = serde_json::json!({
        "seq": seq,
        "at_ms": at_ms,
        "dir": direction.label(),
        "kind": kind,
        "agent": agent,
        "message": message,
        "raw": raw,
    });
    format!("{line}\n")
}

/// The ring the whole process tapes onto.
pub fn tape() -> &'static Tape {
    static TAPE: OnceLock<Tape> = OnceLock::new();
    TAPE.get_or_init(Tape::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tape is process-wide, and nothing else in this crate puts anything on it.
    #[test]
    fn the_tape_keeps_what_crossed_and_skips_the_pane_hot_path() {
        let (hub, host) = hub();
        let client = hub.connect();
        tape().clear();

        client.send(Message::ListProjects);
        client.send(Message::TerminalInput {
            pane_id: PaneId::generate(),
            bytes: b"ls".to_vec(),
        });

        let taped = tape().snapshot();
        assert_eq!(taped.len(), 1, "taped: {taped:?}");
        assert_eq!(taped[0].direction, Direction::Outbound);
        assert_eq!(taped[0].kind, "ListProjects");

        // And the ring is a ring: what does not fit falls off the front.
        for _ in 0..TAPE_CAPACITY + 100 {
            host.send(To::Everyone, Message::ListProjects);
        }
        let (held, dropped) = tape().counts();
        assert_eq!(held, TAPE_CAPACITY);
        assert_eq!(dropped, 101);
        assert_eq!(tape().snapshot()[0].direction, Direction::Inbound);
    }
}
