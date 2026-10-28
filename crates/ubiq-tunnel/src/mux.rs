//! The multiplexer: many [`TunnelStream`]s over one tunnel channel.
//!
//! **Threads.** One reader decodes frames and routes them; it never blocks on anything but its
//! own lock, because every queue it pushes into is unbounded — and bounded anyway by credit, since
//! a peer may not send past the window it was granted. One writer drains a control queue
//! (`open`, `opened`, `refused`, `ack`, `close both`) ahead of a bounded data queue (`data`,
//! `close write`), so an ack is never stuck behind a megabyte of someone else's bytes, and a
//! half-close never overtakes the data before it.
//!
//! **Credit.** A stream's sender holds at most `window` unacknowledged bytes, and the link at most
//! [`TUNNEL_LINK_IN_FLIGHT`] across every stream; [`TunnelStream::send`] blocks until both have
//! room. The receiver's [`TunnelStream::ack`] returns the credit — the consumer calls it once the
//! bytes have left (been written to its socket), so a slow socket slows its producer.

use std::collections::HashMap;
use std::io::{self, BufWriter, Read, Write};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use ubiq_proto::ids::TunnelStreamId;
use ubiq_proto::tunnel::{
    self, TUNNEL_CHUNK_MAX, TUNNEL_LINK_IN_FLIGHT, TUNNEL_STREAM_WINDOW, TUNNEL_VERSION,
    TunnelFrame, TunnelHalf,
};
use ubiq_proto::wire::WireError;

use crate::rate::Bucket;

/// Frames of data the writer holds before a sender blocks. Credit bounds it long before this does.
const DATA_QUEUE: usize = 64;
/// How long an `Open` waits for `Opened` or `Refused`.
const OPEN_TIMEOUT: Duration = Duration::from_secs(30);
/// The writer's buffer: frames are coalesced into one write per drained queue.
const WRITE_BUFFER: usize = 64 * 1024;

/// What a stream's receiving half yields.
#[derive(Debug, PartialEq, Eq)]
pub enum Recv {
    /// The peer's bytes. The consumer owes an [`TunnelStream::ack`] of their length once it has
    /// written them on.
    Data(Vec<u8>),
    /// The peer half-closed: no more data, though this side may still send.
    Eof,
    /// The stream is gone both ways — a reset by either end or the link dropping. `Some` carries
    /// the reason.
    Reset(Option<String>),
}

enum Out {
    Frame(TunnelFrame),
    Shutdown,
}

struct Entry {
    window: u32,
    /// What this side may still send before an ack.
    credit: u32,
    /// Sent and not yet acknowledged — counted in the link's `in_flight`.
    sent_unacked: usize,
    /// Received and not yet acknowledged — what the peer is holding against its window.
    recv_unacked: usize,
    inbound: flume::Sender<Recv>,
    /// The `open` still waiting for `opened` / `refused`.
    reply: Option<flume::Sender<Result<(), String>>>,
    write_closed: bool,
    peer_write_closed: bool,
}

struct State {
    streams: HashMap<TunnelStreamId, Entry>,
    in_flight: usize,
    dead: Option<String>,
}

impl State {
    fn remove(&mut self, id: TunnelStreamId) -> Option<Entry> {
        let entry = self.streams.remove(&id)?;
        self.in_flight = self.in_flight.saturating_sub(entry.sent_unacked);
        Some(entry)
    }
}

type Closer = Box<dyn FnOnce() + Send>;

pub(crate) struct Inner {
    state: Mutex<State>,
    credit: Condvar,
    ctrl: flume::Sender<Out>,
    data: flume::Sender<TunnelFrame>,
    /// `Some` on the end that dials (the drone); dropped when the link dies.
    incoming: Mutex<Option<flume::Sender<Incoming>>>,
    bucket: Option<Mutex<Bucket>>,
    rate_bps: Option<u64>,
    closer: Mutex<Option<Closer>>,
}

/// One tunnel channel, multiplexed. Cheap to clone; every clone is the same link.
#[derive(Clone)]
pub struct Mux {
    inner: Arc<Inner>,
    incoming: Option<flume::Receiver<Incoming>>,
}

impl Mux {
    /// The window's end: it opens streams and refuses any `open` sent to it. `rate_bps` is the
    /// saved host's `tunnel_rate_bps`, stated in the hello so the drone honours it too.
    ///
    /// Blocks for the hello exchange. `closer`, when given, is called once when the link ends —
    /// whichever side ended it — to shut the underlying channel, so a reader blocked on it wakes.
    pub fn window<R, W>(
        reader: R,
        writer: W,
        rate_bps: Option<u64>,
        closer: Option<Closer>,
    ) -> io::Result<Mux>
    where
        R: Read + Send + 'static,
        W: Write + Send + 'static,
    {
        Self::start(reader, writer, rate_bps, false, closer)
    }

    /// The drone's end: it states no rate of its own, honours the window's, and hands each `open`
    /// out through [`Mux::incoming`]. [`crate::serve`] is the usual consumer.
    pub fn drone<R, W>(reader: R, writer: W, closer: Option<Closer>) -> io::Result<Mux>
    where
        R: Read + Send + 'static,
        W: Write + Send + 'static,
    {
        Self::start(reader, writer, None, true, closer)
    }

    fn start<R, W>(
        mut reader: R,
        mut writer: W,
        own_rate: Option<u64>,
        dials: bool,
        closer: Option<Closer>,
    ) -> io::Result<Mux>
    where
        R: Read + Send + 'static,
        W: Write + Send + 'static,
    {
        let peer_rate = match hello(&mut reader, &mut writer, own_rate) {
            Ok(rate) => rate,
            Err(e) => {
                if let Some(closer) = closer {
                    closer();
                }
                return Err(e);
            }
        };
        let rate_bps = match (own_rate, peer_rate) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let (ctrl_tx, ctrl_rx) = flume::unbounded();
        let (data_tx, data_rx) = flume::bounded(DATA_QUEUE);
        let (incoming_tx, incoming_rx) = if dials {
            let (tx, rx) = flume::unbounded();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        let inner = Arc::new(Inner {
            state: Mutex::new(State {
                streams: HashMap::new(),
                in_flight: 0,
                dead: None,
            }),
            credit: Condvar::new(),
            ctrl: ctrl_tx,
            data: data_tx,
            incoming: Mutex::new(incoming_tx),
            bucket: rate_bps.map(|r| Mutex::new(Bucket::new(r))),
            rate_bps,
            closer: Mutex::new(closer),
        });
        let w = inner.clone();
        thread::Builder::new()
            .name("ubiq-tunnel-write".into())
            .spawn(move || write_loop(w, writer, ctrl_rx, data_rx))?;
        let r = inner.clone();
        thread::Builder::new()
            .name("ubiq-tunnel-read".into())
            .spawn(move || read_loop(r, reader))?;
        Ok(Mux {
            inner,
            incoming: incoming_rx,
        })
    }

    /// Open a stream to `host:port` on the far machine. Blocks until the far end dialled — or
    /// refused, or [`OPEN_TIMEOUT`] passed, or the link dropped — and the error is a sentence.
    pub fn open(&self, host: &str, port: u16) -> Result<TunnelStream, String> {
        let id = TunnelStreamId::generate();
        let (reply_tx, reply_rx) = flume::bounded(1);
        let (inbound_tx, inbound_rx) = flume::unbounded();
        {
            let mut st = self.inner.state.lock();
            if let Some(reason) = &st.dead {
                return Err(reason.clone());
            }
            st.streams.insert(
                id,
                Entry::new(TUNNEL_STREAM_WINDOW, inbound_tx, Some(reply_tx)),
            );
        }
        let stream = TunnelStream {
            id,
            inner: self.inner.clone(),
            rx: inbound_rx,
        };
        self.inner.control(TunnelFrame::Open {
            stream: id,
            host: host.to_string(),
            port,
            window: TUNNEL_STREAM_WINDOW,
        });
        match reply_rx.recv_timeout(OPEN_TIMEOUT) {
            Ok(Ok(())) => Ok(stream),
            Ok(Err(error)) => Err(error),
            Err(flume::RecvTimeoutError::Timeout) => Err(format!(
                "the far end did not answer within {}s",
                OPEN_TIMEOUT.as_secs()
            )),
            Err(flume::RecvTimeoutError::Disconnected) => Err(self
                .error()
                .unwrap_or_else(|| "the stream closed before it opened".into())),
        }
    }

    /// The `open`s the far end sent, on the end that dials. `None` on the window's end.
    pub fn incoming(&self) -> Option<flume::Receiver<Incoming>> {
        self.incoming.clone()
    }

    /// Whether the link is still up.
    pub fn is_alive(&self) -> bool {
        self.inner.state.lock().dead.is_none()
    }

    /// Why the link ended, once it has.
    pub fn error(&self) -> Option<String> {
        self.inner.state.lock().dead.clone()
    }

    /// The ceiling this link's sending side runs at: the lower of the two hellos' `rate_bps`.
    pub fn rate_bps(&self) -> Option<u64> {
        self.inner.rate_bps
    }

    /// End the link from this side: every stream resets, the closer runs.
    pub fn close(&self) {
        self.inner
            .die("the tunnel link was closed by this end".into());
    }

    /// Block until the link's rate allows another local read. A no-op when unlimited.
    pub(crate) fn rate_wait(&self) {
        let Some(bucket) = &self.inner.bucket else {
            return;
        };
        loop {
            let delay = bucket.lock().delay();
            if delay.is_zero() || !self.is_alive() {
                return;
            }
            thread::sleep(delay.min(Duration::from_millis(100)));
        }
    }

    /// Charge `bytes` just read against the link's rate.
    pub(crate) fn rate_debit(&self, bytes: usize) {
        if let Some(bucket) = &self.inner.bucket {
            bucket.lock().debit(bytes);
        }
    }
}

/// An `open` the far end sent: answer it with [`Incoming::accept`] once dialled, or
/// [`Incoming::refuse`]. Dropped unanswered, the stream resets.
pub struct Incoming {
    pub host: String,
    pub port: u16,
    stream: TunnelStream,
}

impl Incoming {
    pub fn accept(self) -> TunnelStream {
        self.stream.inner.control(TunnelFrame::Opened {
            stream: self.stream.id,
        });
        self.stream
    }

    pub fn refuse(self, error: String) {
        let id = self.stream.id;
        self.stream.inner.state.lock().remove(id);
        self.stream
            .inner
            .control(TunnelFrame::Refused { stream: id, error });
    }
}

/// One stream on a [`Mux`]. `&self` throughout, so one `Arc` serves a sending thread and a
/// receiving thread. Dropped before both halves closed, it resets.
pub struct TunnelStream {
    id: TunnelStreamId,
    inner: Arc<Inner>,
    rx: flume::Receiver<Recv>,
}

impl TunnelStream {
    pub fn id(&self) -> TunnelStreamId {
        self.id
    }

    /// Send `data`, split into chunks, blocking while the stream's window or the link's in-flight
    /// cap is full.
    pub fn send(&self, mut data: &[u8]) -> io::Result<()> {
        while !data.is_empty() {
            let n = self.inner.reserve(self.id, data.len())?;
            let frame = TunnelFrame::Data {
                stream: self.id,
                bytes: data[..n].to_vec(),
            };
            if self.inner.data.send(frame).is_err() {
                return Err(broken(self.inner.reason()));
            }
            data = &data[n..];
        }
        Ok(())
    }

    /// Half-close: this side sends no more. Ordered after every byte already sent.
    pub fn close_write(&self) {
        {
            let mut st = self.inner.state.lock();
            let Some(entry) = st.streams.get_mut(&self.id) else {
                return;
            };
            if entry.write_closed {
                return;
            }
            entry.write_closed = true;
            if entry.peer_write_closed {
                st.remove(self.id);
            }
        }
        let _ = self.inner.data.send(TunnelFrame::Close {
            stream: self.id,
            half: TunnelHalf::Write,
            error: None,
        });
    }

    /// The next thing the peer sent. Blocks.
    pub fn recv(&self) -> Recv {
        self.rx.recv().unwrap_or(Recv::Reset(None))
    }

    /// Return `bytes` of credit: the consumer has written them on.
    pub fn ack(&self, bytes: usize) {
        if bytes == 0 {
            return;
        }
        if let Some(entry) = self.inner.state.lock().streams.get_mut(&self.id) {
            entry.recv_unacked = entry.recv_unacked.saturating_sub(bytes);
        }
        self.inner.control(TunnelFrame::Ack {
            stream: self.id,
            bytes: bytes as u32,
        });
    }

    /// End the stream both ways, telling the peer why.
    pub fn reset(&self, error: Option<String>) {
        self.inner.reset(self.id, error);
    }
}

impl Drop for TunnelStream {
    fn drop(&mut self) {
        let open = self.inner.state.lock().streams.contains_key(&self.id);
        if open {
            self.inner.reset(self.id, None);
        }
    }
}

impl Entry {
    fn new(
        window: u32,
        inbound: flume::Sender<Recv>,
        reply: Option<flume::Sender<Result<(), String>>>,
    ) -> Self {
        Self {
            window,
            credit: window,
            sent_unacked: 0,
            recv_unacked: 0,
            inbound,
            reply,
            write_closed: false,
            peer_write_closed: false,
        }
    }
}

impl Inner {
    fn control(&self, frame: TunnelFrame) {
        let _ = self.ctrl.send(Out::Frame(frame));
    }

    fn reason(&self) -> String {
        self.state
            .lock()
            .dead
            .clone()
            .unwrap_or_else(|| "the tunnel link closed".into())
    }

    /// Wait for room to send up to `want` bytes on `id`, and take it.
    fn reserve(&self, id: TunnelStreamId, want: usize) -> io::Result<usize> {
        let mut guard = self.state.lock();
        loop {
            {
                let st = &mut *guard;
                if let Some(reason) = &st.dead {
                    return Err(broken(reason.clone()));
                }
                let Some(entry) = st.streams.get_mut(&id) else {
                    return Err(broken("the stream was reset".into()));
                };
                if entry.write_closed {
                    return Err(broken("the stream's sending half is closed".into()));
                }
                let room = TUNNEL_LINK_IN_FLIGHT.saturating_sub(st.in_flight);
                let n = want
                    .min(entry.credit as usize)
                    .min(room)
                    .min(TUNNEL_CHUNK_MAX);
                if n > 0 {
                    entry.credit -= n as u32;
                    entry.sent_unacked += n;
                    st.in_flight += n;
                    return Ok(n);
                }
            }
            self.credit.wait_for(&mut guard, Duration::from_secs(1));
        }
    }

    fn reset(&self, id: TunnelStreamId, error: Option<String>) {
        let entry = self.state.lock().remove(id);
        if let Some(entry) = entry {
            let _ = entry.inbound.send(Recv::Reset(error.clone()));
            self.credit.notify_all();
            self.control(TunnelFrame::Close {
                stream: id,
                half: TunnelHalf::Both,
                error,
            });
        }
    }

    fn on_frame(self: &Arc<Self>, frame: TunnelFrame) {
        match frame {
            // A second hello says nothing new.
            TunnelFrame::Hello { .. } => {}
            TunnelFrame::Open {
                stream,
                host,
                port,
                window,
            } => self.on_open(stream, host, port, window),
            TunnelFrame::Opened { stream } => {
                let mut st = self.state.lock();
                if let Some(reply) = st.streams.get_mut(&stream).and_then(|e| e.reply.take()) {
                    let _ = reply.send(Ok(()));
                }
            }
            TunnelFrame::Refused { stream, error } => {
                let entry = self.state.lock().remove(stream);
                if let Some(reply) = entry.and_then(|e| e.reply) {
                    let _ = reply.send(Err(error));
                }
                self.credit.notify_all();
            }
            TunnelFrame::Data { stream, bytes } => {
                let overran = {
                    let mut st = self.state.lock();
                    let Some(entry) = st.streams.get_mut(&stream) else {
                        return;
                    };
                    if bytes.len() > TUNNEL_CHUNK_MAX
                        || entry.recv_unacked + bytes.len() > entry.window as usize
                        || entry.peer_write_closed
                    {
                        true
                    } else {
                        entry.recv_unacked += bytes.len();
                        let _ = entry.inbound.send(Recv::Data(bytes));
                        false
                    }
                };
                if overran {
                    self.reset(
                        stream,
                        Some("the far end sent past the stream's window".into()),
                    );
                }
            }
            TunnelFrame::Ack { stream, bytes } => {
                let mut guard = self.state.lock();
                let st = &mut *guard;
                if let Some(entry) = st.streams.get_mut(&stream) {
                    let n = (bytes as usize).min(entry.sent_unacked);
                    entry.sent_unacked -= n;
                    st.in_flight = st.in_flight.saturating_sub(n);
                    entry.credit = entry.credit.saturating_add(n as u32).min(entry.window);
                    self.credit.notify_all();
                }
            }
            TunnelFrame::Close {
                stream,
                half: TunnelHalf::Write,
                ..
            } => {
                let mut st = self.state.lock();
                if let Some(entry) = st.streams.get_mut(&stream) {
                    entry.peer_write_closed = true;
                    let _ = entry.inbound.send(Recv::Eof);
                    if entry.write_closed {
                        st.remove(stream);
                    }
                }
            }
            TunnelFrame::Close {
                stream,
                half: TunnelHalf::Both,
                error,
            } => {
                let entry = self.state.lock().remove(stream);
                if let Some(entry) = entry {
                    if let Some(reply) = entry.reply {
                        let _ = reply.send(Err(error
                            .clone()
                            .unwrap_or_else(|| "the far end closed the stream".into())));
                    }
                    let _ = entry.inbound.send(Recv::Reset(error));
                    self.credit.notify_all();
                }
            }
        }
    }

    fn on_open(self: &Arc<Self>, stream: TunnelStreamId, host: String, port: u16, window: u32) {
        let Some(incoming) = self.incoming.lock().clone() else {
            self.control(TunnelFrame::Refused {
                stream,
                error: "this end of the tunnel does not dial".into(),
            });
            return;
        };
        if window == 0 {
            self.control(TunnelFrame::Refused {
                stream,
                error: "an open with no window".into(),
            });
            return;
        }
        let (inbound_tx, inbound_rx) = flume::unbounded();
        {
            let mut st = self.state.lock();
            if st.dead.is_some() || st.streams.contains_key(&stream) {
                return;
            }
            st.streams
                .insert(stream, Entry::new(window, inbound_tx, None));
        }
        let open = Incoming {
            host,
            port,
            stream: TunnelStream {
                id: stream,
                inner: self.clone(),
                rx: inbound_rx,
            },
        };
        if let Err(flume::SendError(open)) = incoming.send(open) {
            open.refuse("the drone is not serving tunnels".into());
        }
    }

    /// The link is over: every stream resets, every waiter wakes, the writer stops, the closer
    /// runs. Idempotent — the first reason wins.
    fn die(&self, reason: String) {
        let streams = {
            let mut st = self.state.lock();
            if st.dead.is_some() {
                return;
            }
            tracing::debug!(target: "tunnel", "link down: {reason}");
            st.dead = Some(reason.clone());
            st.in_flight = 0;
            std::mem::take(&mut st.streams)
        };
        *self.incoming.lock() = None;
        for (_, entry) in streams {
            let _ = entry.inbound.send(Recv::Reset(Some(reason.clone())));
        }
        self.credit.notify_all();
        let _ = self.ctrl.send(Out::Shutdown);
        if let Some(closer) = self.closer.lock().take() {
            closer();
        }
    }
}

fn hello<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    rate_bps: Option<u64>,
) -> io::Result<Option<u64>> {
    tunnel::write_frame(
        writer,
        &TunnelFrame::Hello {
            version: TUNNEL_VERSION,
            rate_bps,
        },
    )
    .map_err(wire_io)?;
    writer.flush()?;
    match tunnel::read_frame(reader).map_err(wire_io)? {
        TunnelFrame::Hello {
            version: TUNNEL_VERSION,
            rate_bps,
        } => Ok(rate_bps),
        TunnelFrame::Hello { version, .. } => Err(io::Error::other(format!(
            "the far end speaks tunnel version {version}, this end {TUNNEL_VERSION}"
        ))),
        _ => Err(io::Error::other(
            "the far end's first tunnel frame was not a hello",
        )),
    }
}

fn write_loop<W: Write>(
    inner: Arc<Inner>,
    writer: W,
    ctrl: flume::Receiver<Out>,
    data: flume::Receiver<TunnelFrame>,
) {
    let mut w = BufWriter::with_capacity(WRITE_BUFFER, writer);
    let result = (|| -> Result<(), String> {
        loop {
            let out = match ctrl.try_recv() {
                Ok(out) => out,
                Err(_) => match data.try_recv() {
                    Ok(frame) => Out::Frame(frame),
                    Err(_) => {
                        w.flush().map_err(|e| format!("tunnel write: {e}"))?;
                        let next = flume::Selector::new()
                            .recv(&ctrl, |r| r.ok())
                            .recv(&data, |r| r.ok().map(Out::Frame))
                            .wait();
                        match next {
                            Some(out) => out,
                            None => return Ok(()),
                        }
                    }
                },
            };
            match out {
                Out::Shutdown => return Ok(()),
                Out::Frame(frame) => {
                    tunnel::write_frame(&mut w, &frame).map_err(|e| format!("tunnel write: {e}"))?
                }
            }
        }
    })();
    let _ = w.flush();
    drop(w);
    inner.die(
        result
            .err()
            .unwrap_or_else(|| "the tunnel link closed".into()),
    );
}

fn read_loop<R: Read>(inner: Arc<Inner>, mut reader: R) {
    let reason = loop {
        match tunnel::read_frame(&mut reader) {
            Ok(frame) => inner.on_frame(frame),
            Err(WireError::Eof) => break "the tunnel link closed".to_string(),
            Err(e) => break format!("tunnel read: {e}"),
        }
    };
    inner.die(reason);
}

fn wire_io(e: WireError) -> io::Error {
    match e {
        WireError::Io(e) => e,
        e => io::Error::other(e.to_string()),
    }
}

fn broken(reason: String) -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, reason)
}
