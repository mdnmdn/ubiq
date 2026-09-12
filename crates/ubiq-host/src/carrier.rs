//! A session over one duplex byte stream: attach a `Client`, and shuttle frames until either
//! side stops.
//!
//! This is the whole of what a client is, whatever carries it. [`pump`] attaches the same
//! `Client` a local window gets from [`Hub::connect`], so every message family works with nothing
//! written here that knows what any of them mean — and no message family is special-cased, nor
//! should one ever be.
//!
//! **It is deliberately not in [`crate::remote`].** That module is the TCP listener and is gated
//! behind the `listener` feature; the pump underneath it needs neither a socket nor TLS. A lean
//! build — the host as a drone links it, with no listener, no version control and no harness
//! library — carries this and gets a session over whatever byte stream it was handed: an `ssh`
//! exec channel's standard input and output, a pipe, a unix socket.
//!
//! What a carrier must supply beyond the two halves is [`Closer`], and the comment on it says why
//! dropping the stream is not enough.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use ubiq_proto::bus::Hub;
use ubiq_proto::carrier::{Beat, Heartbeat};
use ubiq_proto::wire;

/// How often the writer thread wakes to check whether the reader has given up, when nothing has
/// arrived from the coordinator to write. Bounds how long a dead read direction takes to end the
/// whole session; short enough nobody notices, long enough to cost nothing while both sides are
/// healthy.
const STOP_POLL: Duration = Duration::from_millis(200);

/// A stream half the pumps can own: anything readable, writable and sendable across threads.
///
/// An accepted TCP socket hands over itself and its `try_clone`; a TLS session hands over two
/// handles to the same session behind a lock, because rustls has no `try_clone`; a drone over an
/// `ssh` exec channel hands over standard input and standard output, which are already two.
pub trait Socket: Read + Write + Send + 'static {}

impl<T: Read + Write + Send + 'static> Socket for T {}

/// How a carrier forces its read direction to end.
///
/// The writer calls this on its way out, and it has to make a *blocked* `read` on the other half
/// return. Dropping the writer's own handle does not: for a socket the kernel sends no EOF while
/// the reader still holds its clone, and for standard input there is no handle to drop that the
/// reader is not already sitting in. Without it the host never sees `Gone` and never reaps the
/// panes the session left behind.
pub trait Closer: Send + 'static {
    fn close(&self);
}

/// A carrier whose read direction ends on its own, so there is nothing to force.
///
/// Sound only when losing the write direction is guaranteed to end the read one too — a drone on
/// an `ssh` exec channel is the case: the same dead `ssh` that fails the write has already closed
/// the standard input the reader is blocked on, so EOF is already on its way.
pub struct NoCloser;

impl Closer for NoCloser {
    fn close(&self) {}
}

/// The session, once any handshake the carrier needed is done.
///
/// The `Client` is shared behind an `Arc` because both directions need it — the reader calls
/// `send`, the writer drains `from_host` — and both take `&self`. It drops, telling the coordinator
/// `FromClient::Gone`, once both threads have released their half. A blocking `recv()` on the
/// writer's side would not notice the reader giving up (the coordinator has no reason to stop
/// answering just because the stream died), so the writer polls `from_host` instead and checks
/// `stopped`, which the reader sets on its way out.
///
/// Returns when both directions have finished, so a caller whose process exists only to serve one
/// session — a drone — can treat it as the whole of its run.
///
/// **The heartbeat is transport, not contract.** A `Ping` is answered and a `Pong` is swallowed
/// here; neither is ever handed to the hub, on the same standing the HTTP upgrade in
/// [`crate::remote`] has. The rules — the interval, the miss count, what counts as an answer —
/// are `ubiq_proto::carrier`'s, because the interface has its own pump that must not drift from
/// this one.
pub fn pump(reader: Box<dyn Socket>, writer: Box<dyn Socket>, closer: Box<dyn Closer>, hub: Hub) {
    let client = Arc::new(hub.connect());
    let stopped = Arc::new(AtomicBool::new(false));
    let beat = Arc::new(Mutex::new(Heartbeat::new()));
    // Frames the *reader* needs written: a `Pong`, and nothing else. It cannot write the stream
    // itself — the writer owns that half — and it must not reach the hub to get there, because
    // the hub is what this must stay out of.
    let (pongs, pongs_out) = flume::unbounded();

    let mut reader_stream = reader;
    let mut writer_stream = writer;

    let writer_client = client.clone();
    let writer_stopped = stopped.clone();
    let writer_beat = beat.clone();
    let writer = thread::Builder::new()
        .name("ubiq-carrier-writer".to_string())
        .spawn(move || {
            loop {
                // Before waiting, and so also after every frame written: an outbound stream that
                // is busy while the inbound one is dead never reaches the timeout arm below.
                if writer_beat.lock().expect("the heartbeat").is_gone() {
                    tracing::info!("the peer stopped answering: ending the session");
                    break;
                }
                let picked = flume::Selector::new()
                    .recv(&pongs_out, |taken| taken.ok())
                    .recv(writer_client.from_host(), |taken| taken.ok())
                    .wait_timeout(STOP_POLL);
                match picked {
                    Ok(Some(message)) => {
                        if wire::write_frame(&mut writer_stream, &message).is_err() {
                            break;
                        }
                    }
                    // One of the two channels is gone, which for `from_host` is the coordinator
                    // dropping the client and for `pongs` cannot happen while the reader lives.
                    Ok(None) => break,
                    Err(flume::select::SelectError::Timeout) => {
                        if writer_stopped.load(Ordering::Relaxed) {
                            break;
                        }
                        let owed = writer_beat.lock().expect("the heartbeat").due();
                        if let Some(ping) = owed
                            && wire::write_frame(&mut writer_stream, &ping).is_err()
                        {
                            break;
                        }
                    }
                }
            }
            // End the read direction while a handle still exists — see `Closer`.
            closer.close();
            drop(writer_stream);
            drop(writer_client);
        })
        .expect("the carrier writer thread");

    while let Ok(message) = wire::read_frame(&mut reader_stream) {
        let verdict = beat.lock().expect("the heartbeat").inbound(message);
        match verdict {
            Beat::Deliver(message) => client.send(message),
            Beat::Answer(pong) => {
                if pongs.send(pong).is_err() {
                    break;
                }
            }
            Beat::Swallow => {}
        }
    }
    stopped.store(true, Ordering::Relaxed);
    drop(client);

    let _ = writer.join();
}
