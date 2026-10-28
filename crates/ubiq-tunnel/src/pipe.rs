//! A local socket spliced to a [`TunnelStream`]: one thread each way, both ends' half-closes kept.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use parking_lot::Mutex;
use ubiq_proto::tunnel::TUNNEL_CHUNK_MAX;

use crate::mux::{Mux, Recv, TunnelStream};

/// What a request direction may do to the bytes before they enter the tunnel — the HTTP kind's
/// Host rewrite. Fed every read; writes what to send into the vector.
pub(crate) type Transform = Box<dyn FnMut(&[u8], &mut Vec<u8>) -> io::Result<()> + Send>;

/// One spec's running totals, shared by its listener and every connection it accepted.
#[derive(Default)]
pub(crate) struct Counters {
    pub(crate) conns: AtomicU64,
    pub(crate) active: AtomicU64,
    /// Read from the local socket, sent toward the far machine.
    pub(crate) up: AtomicU64,
    /// Received from the far machine, written to the local socket.
    pub(crate) down: AtomicU64,
    pub(crate) last_error: Mutex<Option<String>>,
}

impl Counters {
    pub(crate) fn error(&self, error: String) {
        tracing::debug!(target: "tunnel", "{error}");
        *self.last_error.lock() = Some(error);
    }
}

fn add(counters: &Option<Arc<Counters>>, pick: fn(&Counters) -> &AtomicU64, n: usize) {
    if let Some(c) = counters {
        pick(c).fetch_add(n as u64, Ordering::Relaxed);
    }
}

/// Splice `local` and `stream` until both directions end. Blocks; the local-to-tunnel direction
/// runs on a thread of its own.
pub(crate) fn pipe(
    mux: &Mux,
    local: TcpStream,
    stream: TunnelStream,
    transform: Option<Transform>,
    counters: Option<Arc<Counters>>,
) {
    let stream = Arc::new(stream);
    let up = match local.try_clone() {
        Ok(reader) => {
            let (mux, stream, counters) = (mux.clone(), stream.clone(), counters.clone());
            thread::Builder::new()
                .name("ubiq-tunnel-up".into())
                .spawn(move || pump_up(&mux, reader, &stream, transform, &counters))
                .ok()
        }
        Err(_) => None,
    };
    if up.is_none() {
        stream.reset(Some("could not split the local socket".into()));
    }
    pump_down(local, &stream, &counters);
    if let Some(up) = up {
        let _ = up.join();
    }
}

fn pump_up(
    mux: &Mux,
    mut local: TcpStream,
    stream: &TunnelStream,
    mut transform: Option<Transform>,
    counters: &Option<Arc<Counters>>,
) {
    let mut buf = vec![0u8; TUNNEL_CHUNK_MAX];
    let mut out = Vec::new();
    loop {
        mux.rate_wait();
        let n = match local.read(&mut buf) {
            Ok(0) => {
                stream.close_write();
                return;
            }
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                stream.reset(Some(format!("the local socket failed: {e}")));
                return;
            }
        };
        mux.rate_debit(n);
        add(counters, |c| &c.up, n);
        let bytes = match transform.as_mut() {
            Some(t) => {
                out.clear();
                if let Err(e) = t(&buf[..n], &mut out) {
                    if let Some(c) = counters {
                        c.error(e.to_string());
                    }
                    stream.reset(Some(e.to_string()));
                    crate::http::answer_refusal(&mut local, &e);
                    let _ = local.shutdown(Shutdown::Both);
                    return;
                }
                &out[..]
            }
            None => &buf[..n],
        };
        if stream.send(bytes).is_err() {
            let _ = local.shutdown(Shutdown::Both);
            return;
        }
    }
}

fn pump_down(mut local: TcpStream, stream: &TunnelStream, counters: &Option<Arc<Counters>>) {
    loop {
        match stream.recv() {
            Recv::Data(bytes) => {
                if let Err(e) = local.write_all(&bytes) {
                    stream.reset(Some(format!("the local socket failed: {e}")));
                    let _ = local.shutdown(Shutdown::Both);
                    return;
                }
                add(counters, |c| &c.down, bytes.len());
                stream.ack(bytes.len());
            }
            Recv::Eof => {
                let _ = local.shutdown(Shutdown::Write);
                return;
            }
            Recv::Reset(error) => {
                if let (Some(c), Some(e)) = (counters, error) {
                    c.error(e);
                }
                let _ = local.shutdown(Shutdown::Both);
                return;
            }
        }
    }
}
