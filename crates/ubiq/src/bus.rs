//! The bus: the channel pair the two halves talk over, and the byte streams a pane is handed.
//!
//! Both halves live in one process today, so the bus is a pair of unbounded channels carrying
//! [`Message`] values. Unbounded is the contract, not an accident: a UI that falls behind must
//! never stall the coordinator's reader, because that stalls the harness.
//!
//! The emulator wants a `Read` and a `Write`. It gets [`PaneOutput`] and [`PaneInput`], which are
//! bus endpoints for one pane ID — never a pseudo-terminal. That is what keeps the UI honest
//! about the pane being an ID plus a byte stream.

use std::io::{self, Read, Write};

use uuid::Uuid;

use crate::messages::Message;

/// The UI's end of the bus.
pub struct UiEnd {
    to_coordinator: flume::Sender<Message>,
    /// Messages from the coordinator, drained by the UI's router task.
    pub from_coordinator: flume::Receiver<Message>,
}

/// The coordinator's end of the bus.
pub struct CoordinatorEnd {
    pub to_ui: flume::Sender<Message>,
    pub from_ui: flume::Receiver<Message>,
}

/// Open a bus. One end goes to the window, the other to the coordinator.
pub fn pair() -> (UiEnd, CoordinatorEnd) {
    let (to_coordinator, from_ui) = flume::unbounded();
    let (to_ui, from_coordinator) = flume::unbounded();
    (
        UiEnd {
            to_coordinator,
            from_coordinator,
        },
        CoordinatorEnd { to_ui, from_ui },
    )
}

impl UiEnd {
    /// Say something to the coordinator. A closed bus is not an error the UI can act on, so it is
    /// dropped rather than surfaced.
    pub fn send(&self, message: Message) {
        let _ = self.to_coordinator.send(message);
    }

    /// A sender for the callbacks the emulator invokes on its own — a resize it measured, say —
    /// which need to reach the coordinator without a window in hand.
    pub fn sender(&self) -> flume::Sender<Message> {
        self.to_coordinator.clone()
    }

    /// The write half for one pane: keystrokes leave as [`Message::TerminalInput`].
    pub fn input(&self, pane_id: Uuid) -> PaneInput {
        PaneInput {
            pane_id,
            to_coordinator: self.to_coordinator.clone(),
        }
    }
}

/// The write half handed to a pane's emulator.
pub struct PaneInput {
    pane_id: Uuid,
    to_coordinator: flume::Sender<Message>,
}

impl Write for PaneInput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.to_coordinator
            .send(Message::TerminalInput {
                pane_id: self.pane_id,
                bytes: buf.to_vec(),
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
