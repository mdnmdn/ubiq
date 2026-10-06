//! Held panes, whatever the links arrive on.
//!
//! A drone that outlives its link needs three things and none of them is a transport: one relay
//! that keeps the panes, one hub every attach is a client of, and a countdown that ends the process
//! when nobody has been attached for the linger. [`serve`] is exactly those, fed by an iterator of
//! [`Link`]s — a duplex byte stream each, already past whatever preamble its transport has. The
//! unix socket in [`crate::socket`] is one source of links; anything else that can hand over a
//! reader, a writer and a way to end the read direction is another, and builds on every platform.
//!
//! **Every link is a new host attach**: its own handshake, its own client on the same hub, nothing
//! resumed. The relay re-announces what it is holding, so a reattach needs no state the two ends
//! have to agree about.
//!
//! **Linger is owned by the relay, not by the link source.** The links are consumed on a thread of
//! their own, so an iterator blocked in `next()` waiting for the next attach cannot hold up the
//! countdown: when the relay's timer runs out it kills the panes and returns, and [`serve`] returns
//! with it, leaving the consumer thread (and whatever it is blocked on) to die with the process.
//! That is the one exit, for the reason [`crate::carrier`] gives. Whoever called [`serve`] cleans up
//! what the source created — a socket path, say — after it returns.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use ubiq_host::carrier::{self, Closer};
use ubiq_proto::bus;
use ubiq_proto::carrier::{greet, hello};

use crate::linger::{Linger, Live};
use crate::relay::{Relay, Root};
use crate::state::DroneState;

/// How often the state file is rewritten, so `panes` and `linger` stay roughly true for a reader
/// between attaches. Roughly, not exactly: it is a cache of what the drone says about itself.
const STATE_REFRESH: Duration = Duration::from_secs(5);

/// One attach: a duplex byte stream, ready for the drone's handshake to go out on it.
pub struct Link {
    /// What the client sends.
    pub reader: Box<dyn Read + Send>,
    /// What the drone sends. Written whole frames at a time and flushed by the pump.
    pub writer: Box<dyn Write + Send>,
    /// Forces a blocked `reader` to return when the writer gives up — see [`Closer`].
    pub closer: Box<dyn Closer>,
    /// The linger this attach asks for, re-asserted before anything is served, so changing it
    /// costs no restart.
    pub linger: Option<Linger>,
}

impl Link {
    /// A link with no linger of its own.
    pub fn new(
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        closer: Box<dyn Closer>,
    ) -> Self {
        Self {
            reader,
            writer,
            closer,
            linger: None,
        }
    }
}

/// What [`serve`] shares with the source of its links, and where it records itself.
pub struct Options {
    /// The linger, the pane count and the stop flag. Kept by the caller too: a link source that
    /// understands a stop request calls [`Live::stop`] on it, and the relay treats that as an
    /// immediate expiry.
    pub live: Arc<Live>,
    /// The socket path whose sibling state file ([`crate::state::state_path`]) this drone writes,
    /// refreshes and removes. `None` writes none.
    pub state: Option<PathBuf>,
}

impl Options {
    /// Held with this linger, and no state file.
    pub fn new(linger: Linger) -> Self {
        Self {
            live: Arc::new(Live::new(linger)),
            state: None,
        }
    }
}

/// Hold the panes of `roots` across every link `links` yields, and return when the linger runs
/// out or a stop is asked for.
///
/// The relay runs on its own thread, and the links are consumed on another with one session
/// thread per link, so the calling thread is the one thing that ends the process. See the module
/// documentation for why a blocking iterator cannot hold the linger up. An iterator that ends
/// means "no more links", not "stop": the linger alone decides when `serve` returns.
///
/// `serve` assumes the process exits soon after it returns: threads of links that arrived late
/// are not reaped.
pub fn serve(
    roots: Vec<Root>,
    options: Options,
    links: impl Iterator<Item = Link> + Send + 'static,
) {
    let Options { live, state } = options;

    // The state file only ever names the folders, never the ids bound to them: `DroneState` is
    // what a caller lists and adopts by, and a project's id reaches a client on the wire, once, at
    // `ListProjects`.
    let paths: Vec<PathBuf> = roots.iter().map(|root| root.path.clone()).collect();
    // Dropping the sender wakes the refresh thread, which is then joined before the file goes.
    let mut refresher = None;

    if let Some(socket) = &state {
        if let Err(error) = DroneState::new(socket, &paths, 0, live.linger()).write(socket) {
            tracing::warn!(
                "could not write the state file for {}: {error}",
                socket.display()
            );
        }
        let (stop, stopped) = mpsc::channel::<()>();
        let handle = thread::Builder::new()
            .name("ubiq-drone-state".to_string())
            .spawn({
                let (socket, live) = (socket.clone(), live.clone());
                move || refresh_state(&socket, &paths, &live, &stopped)
            })
            .expect("the drone state thread");
        refresher = Some((stop, handle));
    }

    let (hub, host) = bus::hub();
    let relay = Relay::holding(roots, live.clone());
    let relay = thread::Builder::new()
        .name("ubiq-drone-relay".to_string())
        .spawn(move || relay.run(host))
        .expect("the drone relay thread");

    thread::Builder::new()
        .name("ubiq-drone-links".to_string())
        .spawn({
            let live = live.clone();
            let hub = hub.clone();
            move || {
                for link in links {
                    let (hub, live) = (hub.clone(), live.clone());
                    thread::Builder::new()
                        .name("ubiq-drone-session".to_string())
                        .spawn(move || serve_one(link, hub, &live))
                        .expect("the drone session thread");
                }
            }
        })
        .expect("the drone links thread");

    // `hub` stays alive here until the relay is done, so a links iterator that ends means only
    // "no more links": the relay never sees the hub disconnect, and the linger alone decides.
    let _ = relay.join();
    drop(hub);
    if let Some((stop, handle)) = refresher {
        drop(stop);
        let _ = handle.join();
    }
    if let Some(socket) = &state {
        DroneState::remove(socket);
    }
}

/// Rewrite the state file every [`STATE_REFRESH`] until `stopped` disconnects or the socket path
/// is gone. [`serve`] joins it before removing the file, so a refresh can never resurrect it.
fn refresh_state(
    socket: &std::path::Path,
    roots: &[PathBuf],
    live: &Live,
    stopped: &mpsc::Receiver<()>,
) {
    loop {
        if !matches!(
            stopped.recv_timeout(STATE_REFRESH),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) || !socket.exists()
        {
            return;
        }
        let state = DroneState::new(socket, roots, live.panes(), live.linger());
        if let Err(error) = state.write(socket) {
            tracing::warn!(
                "could not refresh the state file for {}: {error}",
                socket.display()
            );
        }
    }
}

fn serve_one(link: Link, hub: bus::Hub, live: &Live) {
    let Link {
        reader,
        writer,
        closer,
        linger,
    } = link;
    // Re-asserted before anything else: a drone about to hold panes should already know for how
    // long.
    if let Some(asked) = linger {
        tracing::info!("the attaching client asked for linger {asked}");
        live.set_linger(asked);
    }
    let (mut reader, mut writer) = (ReadOnly(reader), WriteOnly(writer));
    let introduction = hello(env!("CARGO_PKG_VERSION"), &crate::capabilities());
    if let Err(refusal) = greet(&mut reader, &mut writer, &introduction) {
        tracing::warn!("the attaching client refused this drone: {refusal}");
        return;
    }
    carrier::pump(Box::new(reader), Box::new(writer), closer, hub);
    tracing::info!("an attached client has gone");
}

/// A read half as a [`carrier::Socket`]; writing to it is a wiring mistake and says so.
struct ReadOnly(Box<dyn Read + Send>);

impl Read for ReadOnly {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for ReadOnly {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// A write half as a [`carrier::Socket`], the mirror of [`ReadOnly`].
struct WriteOnly(Box<dyn Write + Send>);

impl Read for WriteOnly {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

impl Write for WriteOnly {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
