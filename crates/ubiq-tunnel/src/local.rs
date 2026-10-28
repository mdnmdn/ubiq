//! The window's half: a loopback listener per enabled [`TunnelSpec`], spliced to the attached mux.
//!
//! Listeners are bound once, at [`TunnelManager::start`], and survive the link: a connection
//! accepted with no live mux under them is refused with a sentence — a `502` for the HTTP kind, a
//! close for TCP — and recorded as the spec's last error, until [`TunnelManager::attach`] gives
//! them a new one. The HTTP kind answers `403` to a request whose `Host` is not the loopback — the
//! DNS-rebinding guard in [`crate::http`].

use std::io::{Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use parking_lot::Mutex;
use rustls::pki_types::ServerName;
use ubiq_proto::settings::{TunnelKind, TunnelSpec};

use crate::http::HostRewriter;
use crate::mux::Mux;
use crate::pipe::{self, Counters};
use crate::tls;

/// How often an idle listener looks at the stop flag.
const ACCEPT_POLL: Duration = Duration::from_millis(50);

/// One spec's state, as the settings page or a status line would draw it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TunnelStatus {
    /// The [`TunnelSpec::id`].
    pub id: String,
    pub local_port: u16,
    /// Whether the loopback listener is bound.
    pub listening: bool,
    /// Whether a live link is attached under it.
    pub linked: bool,
    /// Connections accepted, ever.
    pub conns: u64,
    /// Connections open now.
    pub active: u64,
    /// Bytes read from local clients, sent toward the far machine.
    pub bytes_up: u64,
    /// Bytes from the far machine written to local clients.
    pub bytes_down: u64,
    /// The latest failure — a bind, a refused dial, a dropped link — as a sentence.
    pub last_error: Option<String>,
}

struct Tunnel {
    spec: TunnelSpec,
    counters: Arc<Counters>,
    listening: AtomicBool,
}

struct Shared {
    mux: Mutex<Option<Mux>>,
    stop: AtomicBool,
}

/// Every forwarded port of one saved host. Dropped, it stops listening and closes its link.
pub struct TunnelManager {
    shared: Arc<Shared>,
    tunnels: Vec<Arc<Tunnel>>,
    /// The accept loops, joined by [`Self::stop`] so the ports are free when it returns.
    listeners: Mutex<Vec<thread::JoinHandle<()>>>,
}

impl TunnelManager {
    /// Bind `127.0.0.1:local_port` for every enabled spec. A port that will not bind is reported
    /// in that spec's status and does not stop the others. `mux` may come later, by
    /// [`Self::attach`].
    pub fn start(specs: &[TunnelSpec], mux: Option<Mux>) -> TunnelManager {
        let shared = Arc::new(Shared {
            mux: Mutex::new(mux),
            stop: AtomicBool::new(false),
        });
        let mut tunnels = Vec::new();
        let mut listeners = Vec::new();
        for spec in specs.iter().filter(|s| s.enabled) {
            let tunnel = Arc::new(Tunnel {
                spec: spec.clone(),
                counters: Arc::new(Counters::default()),
                listening: AtomicBool::new(false),
            });
            match TcpListener::bind((Ipv4Addr::LOCALHOST, spec.local_port)) {
                Ok(listener) => {
                    tunnel.listening.store(true, Ordering::Relaxed);
                    let (shared, tunnel) = (shared.clone(), tunnel.clone());
                    let spawned = thread::Builder::new()
                        .name(format!("ubiq-tunnel-listen-{}", spec.local_port))
                        .spawn(move || accept_loop(shared, tunnel, listener));
                    match spawned {
                        Ok(handle) => listeners.push(handle),
                        Err(e) => {
                            tunnels.push(fail(spec, format!("could not start the listener: {e}")));
                            continue;
                        }
                    }
                }
                Err(e) => tunnel.counters.error(format!(
                    "could not listen on 127.0.0.1:{}: {e}",
                    spec.local_port
                )),
            }
            tunnels.push(tunnel);
        }
        TunnelManager {
            shared,
            tunnels,
            listeners: Mutex::new(listeners),
        }
    }

    /// Put `mux` under every listener, closing the one it replaces.
    pub fn attach(&self, mux: Mux) {
        let old = self.shared.mux.lock().replace(mux);
        if let Some(old) = old {
            old.close();
        }
    }

    /// Close the link; the listeners stay and refuse until the next [`Self::attach`].
    pub fn detach(&self) {
        let old = self.shared.mux.lock().take();
        if let Some(old) = old {
            old.close();
        }
    }

    /// Whether a live link is attached.
    pub fn linked(&self) -> bool {
        self.shared.mux.lock().as_ref().is_some_and(Mux::is_alive)
    }

    /// Every enabled spec's state, in the order given to [`Self::start`].
    pub fn status(&self) -> Vec<TunnelStatus> {
        let linked = self.linked();
        self.tunnels
            .iter()
            .map(|t| TunnelStatus {
                id: t.spec.id.clone(),
                local_port: t.spec.local_port,
                listening: t.listening.load(Ordering::Relaxed),
                linked,
                conns: t.counters.conns.load(Ordering::Relaxed),
                active: t.counters.active.load(Ordering::Relaxed),
                bytes_up: t.counters.up.load(Ordering::Relaxed),
                bytes_down: t.counters.down.load(Ordering::Relaxed),
                last_error: t.counters.last_error.lock().clone(),
            })
            .collect()
    }

    /// Stop listening and close the link. Connections in flight reset with it. Returns once every
    /// listener is closed — within one [`ACCEPT_POLL`] — so a new manager can bind the same ports
    /// straight away.
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.detach();
        let listeners = std::mem::take(&mut *self.listeners.lock());
        for handle in listeners {
            let _ = handle.join();
        }
    }
}

impl Drop for TunnelManager {
    fn drop(&mut self) {
        self.stop();
    }
}

fn fail(spec: &TunnelSpec, error: String) -> Arc<Tunnel> {
    let counters = Arc::new(Counters::default());
    counters.error(error);
    Arc::new(Tunnel {
        spec: spec.clone(),
        counters,
        listening: AtomicBool::new(false),
    })
}

fn accept_loop(shared: Arc<Shared>, tunnel: Arc<Tunnel>, listener: TcpListener) {
    if let Err(e) = listener.set_nonblocking(true) {
        tunnel.counters.error(format!("the listener failed: {e}"));
    }
    while !shared.stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((socket, _)) => {
                // An accepted socket inherits the listener's non-blocking flag on some platforms.
                let _ = socket.set_nonblocking(false);
                let _ = socket.set_nodelay(true);
                let (shared, tunnel) = (shared.clone(), tunnel.clone());
                let _ = thread::Builder::new()
                    .name("ubiq-tunnel-conn".into())
                    .spawn(move || connection(&shared, &tunnel, socket));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(ACCEPT_POLL),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => {
                tunnel.counters.error(format!("the listener failed: {e}"));
                thread::sleep(ACCEPT_POLL * 4);
            }
        }
    }
    tunnel.listening.store(false, Ordering::Relaxed);
}

struct Active<'a>(&'a Counters);

impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

fn connection(shared: &Shared, tunnel: &Tunnel, socket: TcpStream) {
    let spec = &tunnel.spec;
    let counters = &tunnel.counters;
    counters.conns.fetch_add(1, Ordering::Relaxed);
    counters.active.fetch_add(1, Ordering::Relaxed);
    let _active = Active(counters);

    let mux = shared.mux.lock().clone().filter(Mux::is_alive);
    let Some(mux) = mux else {
        refuse(
            socket,
            spec.kind,
            "the tunnel link is down: no drone is attached to forward this port".into(),
            counters,
        );
        return;
    };
    let stream = match mux.open(&spec.host, spec.port) {
        Ok(stream) => stream,
        Err(e) => {
            refuse(
                socket,
                spec.kind,
                format!("{}:{} was refused: {e}", spec.host, spec.port),
                counters,
            );
            return;
        }
    };
    match spec.kind {
        TunnelKind::Tcp => pipe::pipe(&mux, socket, stream, None, Some(counters.clone())),
        TunnelKind::Http => {
            let mut rewriter = HostRewriter::new(host_header(spec));
            if spec.upstream_tls {
                let server = match ServerName::try_from(spec.host.clone()) {
                    Ok(name) => name,
                    Err(e) => {
                        stream.reset(None);
                        refuse(
                            socket,
                            spec.kind,
                            format!("{} is not a TLS server name: {e}", spec.host),
                            counters,
                        );
                        return;
                    }
                };
                tls::pipe_tls(
                    &mux,
                    socket,
                    stream,
                    rewriter,
                    tls::client_config(spec.insecure),
                    server,
                    counters.clone(),
                );
            } else {
                let transform: pipe::Transform =
                    Box::new(move |input, out| rewriter.feed(input, out));
                pipe::pipe(
                    &mux,
                    socket,
                    stream,
                    Some(transform),
                    Some(counters.clone()),
                );
            }
        }
    }
}

/// The `Host` an HTTP spec sends: the rewrite, or `host[:port]` with the scheme's port left off.
fn host_header(spec: &TunnelSpec) -> String {
    if let Some(rewrite) = spec
        .host_rewrite
        .as_deref()
        .filter(|h| !h.trim().is_empty())
    {
        return rewrite.trim().to_string();
    }
    let host = if spec.host.contains(':') && !spec.host.starts_with('[') {
        format!("[{}]", spec.host)
    } else {
        spec.host.clone()
    };
    let default_port = if spec.upstream_tls { 443 } else { 80 };
    if spec.port == default_port {
        host
    } else {
        format!("{host}:{}", spec.port)
    }
}

/// Turn a local connection away, saying why: a `502` for HTTP, a plain close for TCP.
fn refuse(mut socket: TcpStream, kind: TunnelKind, error: String, counters: &Counters) {
    counters.error(error.clone());
    if kind == TunnelKind::Http {
        let body = format!("ubiq tunnel: {error}\n");
        let head = format!(
            "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain; charset=utf-8\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = socket.write_all(head.as_bytes());
        let _ = socket.write_all(body.as_bytes());
    }
    let _ = socket.shutdown(Shutdown::Write);
    // Drain what the client already sent, so the close is a FIN and not a reset that eats the
    // answer.
    let _ = socket.set_read_timeout(Some(Duration::from_millis(500)));
    let mut sink = [0u8; 4096];
    while matches!(socket.read(&mut sink), Ok(n) if n > 0) {}
    let _ = socket.shutdown(Shutdown::Both);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(host: &str, port: u16, tls: bool, rewrite: Option<&str>) -> TunnelSpec {
        TunnelSpec {
            id: "t".into(),
            kind: TunnelKind::Http,
            local_port: 0,
            host: host.into(),
            port,
            upstream_tls: tls,
            host_rewrite: rewrite.map(Into::into),
            insecure: false,
            enabled: true,
        }
    }

    #[test]
    fn the_host_header_leaves_the_scheme_port_off_and_brackets_v6() {
        assert_eq!(host_header(&spec("api.x", 443, true, None)), "api.x");
        assert_eq!(host_header(&spec("api.x", 443, false, None)), "api.x:443");
        assert_eq!(host_header(&spec("::1", 8080, false, None)), "[::1]:8080");
        assert_eq!(
            host_header(&spec("10.0.0.1", 80, false, Some("svc"))),
            "svc"
        );
    }
}
