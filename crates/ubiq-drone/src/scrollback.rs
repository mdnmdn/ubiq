//! Where a pane's output goes when nobody is attached to see it.
//!
//! A detached drone holds live pseudo-terminals whose clients have gone, and two things follow
//! from that. The reader thread behind each pane must not stop — `Pty::forward_output` gives up
//! the moment its sink says the destination has gone, which is right for a window that closed and
//! wrong for a link that dropped — and the bytes it reads in the meantime have to land somewhere
//! a reattaching client can be shown, or the user gets a live-but-blank terminal until the next
//! keystroke happens to produce output.
//!
//! So every pane's reader and reaper are given a sink that **never** goes away: a private hub
//! whose one client this module holds for the life of the process. One thread drains it, keeps a
//! bounded ring per pane, and forwards to whoever is attached right now — nobody, between links.
//!
//! The ring is [`RING_BYTES`] and drops the oldest first. It is a screen's worth of history and
//! not a transcript: a drone is a guest on somebody else's machine, and the bound is what keeps a
//! forgotten pane from being a memory leak with a process attached to it.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::thread;

use ubiq_proto::bus::{self, Mailbox, To};
use ubiq_proto::ids::PaneId;
use ubiq_proto::messages::Message;

/// What one pane's ring holds: 256 KiB, oldest first.
pub const RING_BYTES: usize = 256 * 1024;

/// One pane's recent output.
#[derive(Default)]
struct Ring {
    bytes: VecDeque<u8>,
}

impl Ring {
    fn push(&mut self, chunk: &[u8]) {
        // A single chunk larger than the ring keeps only its tail — oldest first holds inside a
        // chunk exactly as it holds across them.
        let tail = if chunk.len() > RING_BYTES {
            &chunk[chunk.len() - RING_BYTES..]
        } else {
            chunk
        };
        self.bytes.extend(tail.iter().copied());
        let over = self.bytes.len().saturating_sub(RING_BYTES);
        self.bytes.drain(..over);
    }

    fn snapshot(&self) -> Vec<u8> {
        self.bytes.iter().copied().collect()
    }
}

#[derive(Default)]
struct State {
    /// The attached client's inbox, or nothing between links.
    current: Mutex<Option<Mailbox>>,
    rings: Mutex<HashMap<PaneId, Ring>>,
    /// Panes whose process has ended, told to the relay the next time it asks. Nobody may be
    /// attached when a pane exits, so the message alone cannot be what retires it.
    ended: Mutex<Vec<PaneId>>,
    /// Whether to keep a ring at all. An attached drone kills its panes when its one client
    /// leaves, so nothing it could replay would ever be replayed.
    retain: bool,
}

impl State {
    fn absorb(&self, message: Message) {
        match &message {
            Message::TerminalOutput { pane_id, bytes } if self.retain => {
                self.rings
                    .lock()
                    .expect("the rings")
                    .entry(*pane_id)
                    .or_default()
                    .push(bytes);
            }
            Message::PaneExited { pane_id, .. } => {
                self.rings.lock().expect("the rings").remove(pane_id);
                self.ended.lock().expect("the ended panes").push(*pane_id);
            }
            _ => {}
        }
        let current = self.current.lock().expect("the attached client").clone();
        if let Some(mailbox) = current {
            mailbox.send(message);
        }
    }
}

/// The sink every pane's reader writes into, and the history behind it.
pub struct Scrollback {
    sink: Mailbox,
    state: Arc<State>,
}

impl Scrollback {
    /// Start the fan-out. `retain` decides whether a ring is kept per pane — only a drone that
    /// outlives its client has anything to replay.
    pub fn start(retain: bool) -> Self {
        let (hub, end) = bus::hub();
        let client = hub.connect();
        let sink = end.mailbox(To::Client(client.id()));
        let state = Arc::new(State {
            retain,
            ..State::default()
        });

        let mine = state.clone();
        thread::Builder::new()
            .name("ubiq-drone-scrollback".to_string())
            .spawn(move || {
                // Held for the life of the thread on purpose: the sink above is a clone of this
                // client's sender, and a dropped client is a sink that reports its destination
                // gone — which is exactly what stops a pane's reader.
                let _hub = hub;
                let _end = end;
                while let Ok(message) = client.from_host().recv() {
                    mine.absorb(message);
                }
            })
            .expect("the drone scrollback thread");

        Self { sink, state }
    }

    /// What a pane's reader and reaper are given instead of the client's own mailbox.
    pub fn sink(&self) -> Mailbox {
        self.sink.clone()
    }

    /// A client is here: send it everything from now on.
    pub fn attach(&self, mailbox: Mailbox) {
        *self.state.current.lock().expect("the attached client") = Some(mailbox);
    }

    /// Nobody is here. Output keeps being read and keeps landing in the rings.
    pub fn detach(&self) {
        *self.state.current.lock().expect("the attached client") = None;
    }

    /// One pane's recent output, for a client that has just attached.
    pub fn replay(&self, pane_id: PaneId) -> Vec<u8> {
        self.state
            .rings
            .lock()
            .expect("the rings")
            .get(&pane_id)
            .map(Ring::snapshot)
            .unwrap_or_default()
    }

    /// Forget a pane the relay has closed.
    pub fn forget(&self, pane_id: PaneId) {
        self.state.rings.lock().expect("the rings").remove(&pane_id);
    }

    /// Every pane whose process has ended since this was last asked.
    pub fn ended(&self) -> Vec<PaneId> {
        std::mem::take(&mut *self.state.ended.lock().expect("the ended panes"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_is_bounded_and_drops_the_oldest_first() {
        let mut ring = Ring::default();
        ring.push(b"oldest");
        ring.push(&vec![b'x'; RING_BYTES]);
        let kept = ring.snapshot();
        assert_eq!(
            kept.len(),
            RING_BYTES,
            "the ring never grows past its bound"
        );
        assert!(
            !kept.starts_with(b"oldest"),
            "the oldest bytes are the ones that go"
        );

        // One chunk larger than the whole ring keeps its tail, not its head.
        let mut huge = Ring::default();
        let mut chunk = vec![b'a'; RING_BYTES];
        chunk.extend_from_slice(b"tail");
        huge.push(&chunk);
        let kept = huge.snapshot();
        assert_eq!(kept.len(), RING_BYTES);
        assert!(kept.ends_with(b"tail"));
    }
}
