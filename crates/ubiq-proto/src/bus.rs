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

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

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
#[derive(Debug)]
pub enum FromClient {
    Connected(ClientId),
    Said { client: ClientId, message: Message },
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
            clients: self.clients.clone(),
        }
    }
}

impl HostEnd {
    /// The host's run loop reads here. Blocks; ends when the hub and every client have gone.
    pub fn recv(&self) -> Result<FromClient, flume::RecvError> {
        self.inbox.recv()
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
pub struct Client {
    id: ClientId,
    to_host: flume::Sender<FromClient>,
    from_host: flume::Receiver<Message>,
    clients: Clients,
}

impl Client {
    pub fn id(&self) -> ClientId {
        self.id
    }

    /// Say something to the host. A closed bus is not an error the UI can act on, so it is dropped
    /// rather than surfaced.
    pub fn send(&self, message: Message) {
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
impl Drop for Client {
    fn drop(&mut self) {
        self.clients.lock().remove(&self.id);
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
