//! Standard input and standard output as the carrier.
//!
//! This is the whole transport: the drone is handed one duplex byte stream by whatever started it
//! — an `ssh` exec channel today — and [`ubiq_host::carrier::pump`] is what a session on it is.
//! Nothing here frames, routes or interprets anything; that is all in the pump and in the wire.
//!
//! **EOF on standard input is the exit.** The pump returns when both directions have finished,
//! the hub goes with it, and the relay's run loop ends and kills every pane it still holds. That
//! chain is the one thing this file exists to guarantee: a drone runs on somebody else's machine,
//! and a pseudo-terminal orphaned there is a process nobody knows to go and kill. There is no
//! other shutdown path, deliberately — no signal handler, no timeout, no reconnect — because a
//! second path is a second chance to leave a pane running.
//!
//! Note also what is *not* here: nothing writes to standard output but the pump. Diagnostics go
//! to standard error (see `ubiq_proto::log`), because a stray line on standard output is a corrupt
//! frame and a dead session.

use std::io::{self, Read, Write};
use std::thread;

use ubiq_host::carrier::{self, Closer, NoCloser, Socket};
use ubiq_proto::bus;
use ubiq_proto::carrier::{HandshakeError, greet, hello};

use crate::relay::Relay;

/// Serve one session on standard input and standard output, and return when it ends.
///
/// [`NoCloser`] is sound here for the reason its own documentation gives: the two halves are
/// separate handles of one dead connection, so whatever fails the write has already closed the
/// input the reader is blocked on.
pub fn serve_stdio(relay: Relay) -> Result<(), HandshakeError> {
    serve(
        Box::new(ReadHalf(io::stdin())),
        Box::new(WriteHalf(io::stdout())),
        Box::new(NoCloser),
        relay,
    )
}

/// Introduce this drone, and — if Ubiq says to — serve one session on the stream, returning when
/// it ends.
///
/// **The handshake happens before anything else exists.** The hello goes out and the answer is
/// waited for on the bare stream, ahead of the hub, the relay thread and the pump, so a refusal
/// costs a process that spawned no pseudo-terminal, opened no scratch directory and left nothing
/// on the far machine to clean up. That ordering is the whole point of doing it here rather than
/// as the relay's first message: by the time the relay exists it is too late to have not started.
///
/// The frames are ordinary frames on the existing framing — the stream is already length-prefixed
/// MessagePack, and a second encoding on it would be a second thing to get wrong — and they are
/// consumed here, so neither the hub nor the relay ever sees one.
pub fn serve(
    mut reader: Box<dyn Socket>,
    mut writer: Box<dyn Socket>,
    closer: Box<dyn Closer>,
    relay: Relay,
) -> Result<(), HandshakeError> {
    let introduction = hello(env!("CARGO_PKG_VERSION"), crate::CAPABILITIES);
    greet(&mut reader, &mut writer, &introduction)?;

    let (hub, host) = bus::hub();
    let loop_thread = thread::Builder::new()
        .name("ubiq-drone-relay".to_string())
        .spawn(move || relay.run(host))
        .expect("the drone relay thread");

    carrier::pump(reader, writer, closer, hub);

    // The pump has taken the hub and dropped it, and the client with it. The relay sees `Gone`,
    // then the closed bus, and kills every pane on its way out — joining is what makes that
    // finish before the process does.
    let _ = loop_thread.join();
    Ok(())
}

/// Standard input as a [`carrier::Socket`].
///
/// The trait wants both halves on one handle because a socket has both; standard input has only
/// one, so the other is a hard error rather than a silent success. Writing to the read half would
/// be a wiring mistake, and `pump` never does it — the type says so instead of trusting it.
struct ReadHalf(io::Stdin);

impl Read for ReadHalf {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for ReadHalf {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// Standard output as a [`carrier::Socket`], the mirror of [`ReadHalf`].
///
/// **Every write is flushed.** `pump` writes a frame and moves on, which is right for a socket and
/// wrong for standard output: the standard library buffers it by *line*, and a length-prefixed
/// binary frame contains no newline, so an unflushed answer would sit in the buffer until enough
/// output piled up behind it — a session that appears to hang while working perfectly.
struct WriteHalf(io::Stdout);

impl Read for WriteHalf {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

impl Write for WriteHalf {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.0.write(bytes)?;
        self.0.flush()?;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
