//! `upstream_tls`: the window speaks TLS to the far server itself, so plain `http://localhost`
//! reaches an `https://` upstream. The TLS session ends here, not on the drone — the drone pipes
//! ciphertext and never holds a key.
//!
//! A rustls client connection cannot be read on one thread and written on another, so one thread
//! owns it and two feeders — the local socket, the tunnel stream — post to it.
//!
//! **That thread never blocks on credit.** Its records go to a sender thread of their own, which
//! is the one that waits in [`TunnelStream::send`]; the owner keeps turning the server's records
//! into acks meanwhile. Were the owner to wait for credit itself, two-way traffic — an upload, a
//! websocket — would deadlock: its send waits for the drone's ack, the drone waits on a server
//! that waits for its response to be read, and the response sits unread behind the send. The
//! upload is held back instead at the local feeder, which reads nothing more while
//! [`SEND_BACKLOG`] bytes of records wait for the sender.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, OnceLock};
use std::thread;

use parking_lot::{Condvar, Mutex};
use rustls::ClientConnection;
use rustls::pki_types::ServerName;
use ubiq_proto::tunnel::TUNNEL_CHUNK_MAX;

use crate::http::{RequestRewriter, ResponseRewriter};
use crate::mux::{Mux, Recv, TunnelStream};
use crate::pipe::Counters;

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    static PROVIDER: OnceLock<Arc<rustls::crypto::CryptoProvider>> = OnceLock::new();
    PROVIDER
        .get_or_init(|| Arc::new(rustls::crypto::ring::default_provider()))
        .clone()
}

/// The platform's store first, the bundled public roots as the floor — the dialer's order.
fn roots() -> Arc<rustls::RootCertStore> {
    static ROOTS: OnceLock<Arc<rustls::RootCertStore>> = OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let mut store = rustls::RootCertStore::empty();
            let native = rustls_native_certs::load_native_certs();
            for error in &native.errors {
                tracing::debug!(target: "tunnel", "a platform trust root was not read: {error}");
            }
            store.add_parsable_certificates(native.certs);
            store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(store)
        })
        .clone()
}

pub(crate) fn client_config(insecure: bool) -> Arc<rustls::ClientConfig> {
    static CHAINED: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    static INSECURE: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    let slot = if insecure { &INSECURE } else { &CHAINED };
    slot.get_or_init(|| {
        let builder = rustls::ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .expect("the TLS protocol versions");
        let config = if insecure {
            builder
                .dangerous()
                .with_custom_certificate_verifier(TrustAny::new())
                .with_no_client_auth()
        } else {
            builder
                .with_root_certificates(roots())
                .with_no_client_auth()
        };
        Arc::new(config)
    })
    .clone()
}

/// `insecure`: any certificate chain, any name; handshake signatures still checked.
#[derive(Debug)]
struct TrustAny {
    inner: Arc<rustls::client::WebPkiServerVerifier>,
}

impl TrustAny {
    fn new() -> Arc<Self> {
        let inner =
            rustls::client::WebPkiServerVerifier::builder_with_provider(roots(), provider())
                .build()
                .expect("the web pki verifier");
        Arc::new(Self { inner })
    }
}

impl rustls::client::danger::ServerCertVerifier for TrustAny {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

enum Event {
    Local(Vec<u8>),
    LocalEof,
    LocalError(String),
    Remote(Vec<u8>),
    RemoteEof,
    RemoteReset(Option<String>),
}

/// The most bytes of TLS records waiting for the sender thread before the local feeder stops
/// reading.
const SEND_BACKLOG: usize = 64 * 1024;

/// What the sender thread does, in order.
enum Outbound {
    Records(Vec<u8>),
    CloseWrite,
}

#[derive(Default)]
struct Gate {
    pending: usize,
    /// The sender thread is gone: nothing will drain `pending` any more.
    closed: bool,
}

/// Records handed to the sender thread and not yet sent, in bytes.
#[derive(Default)]
struct Backlog {
    gate: Mutex<Gate>,
    room: Condvar,
}

impl Backlog {
    fn add(&self, n: usize) {
        self.gate.lock().pending += n;
    }

    fn release(&self, n: usize) {
        let mut gate = self.gate.lock();
        gate.pending = gate.pending.saturating_sub(n);
        self.room.notify_all();
    }

    fn close(&self) {
        self.gate.lock().closed = true;
        self.room.notify_all();
    }

    /// Wait until the backlog has room. `false` once the sender is gone.
    fn wait_room(&self) -> bool {
        let mut gate = self.gate.lock();
        while gate.pending >= SEND_BACKLOG && !gate.closed {
            self.room.wait(&mut gate);
        }
        !gate.closed
    }
}

/// The sender thread: the only place a TLS pipe waits for credit.
fn send_loop(
    stream: &TunnelStream,
    ops: flume::Receiver<Outbound>,
    backlog: &Backlog,
) -> io::Result<()> {
    let result = (|| {
        for op in ops.iter() {
            match op {
                Outbound::Records(records) => {
                    stream.send(&records)?;
                    backlog.release(records.len());
                }
                Outbound::CloseWrite => stream.close_write(),
            }
        }
        Ok(())
    })();
    backlog.close();
    result
}

/// Hand every record rustls has ready to the sender thread.
fn flush(
    conn: &mut ClientConnection,
    out: &flume::Sender<Outbound>,
    backlog: &Backlog,
) -> Result<(), String> {
    let mut records = Vec::new();
    while conn.wants_write() {
        conn.write_tls(&mut records)
            .map_err(|e| format!("TLS: {e}"))?;
    }
    if records.is_empty() {
        return Ok(());
    }
    backlog.add(records.len());
    out.send(Outbound::Records(records))
        .map_err(|_| "the tunnel stream ended".to_string())
}

/// [`crate::pipe::pipe`] with a TLS client between the local socket and the stream. Blocks.
pub(crate) fn pipe_tls(
    mux: &Mux,
    mut local: TcpStream,
    stream: TunnelStream,
    rewriters: (RequestRewriter, ResponseRewriter),
    config: Arc<rustls::ClientConfig>,
    server: ServerName<'static>,
    counters: Arc<Counters>,
) {
    let stream = Arc::new(stream);
    let result = run(
        mux, &mut local, &stream, rewriters, config, server, &counters,
    );
    match result {
        Ok(()) => stream.close_write(),
        Err(e) => {
            counters.error(e.clone());
            stream.reset(Some(e));
            let _ = local.shutdown(Shutdown::Both);
        }
    }
}

fn run(
    mux: &Mux,
    local: &mut TcpStream,
    stream: &Arc<TunnelStream>,
    (mut rewriter, mut response): (RequestRewriter, ResponseRewriter),
    config: Arc<rustls::ClientConfig>,
    server: ServerName<'static>,
    counters: &Arc<Counters>,
) -> Result<(), String> {
    let mut conn = ClientConnection::new(config, server).map_err(|e| format!("TLS: {e}"))?;
    // The local feeder is already bounded by its queue; rustls' own cap would refuse early data.
    conn.set_buffer_limit(None);
    let (tx, rx) = flume::bounded::<Event>(8);
    let backlog = Arc::new(Backlog::default());
    let (out_tx, out_rx) = flume::unbounded::<Outbound>();

    let (send_stream, send_backlog) = (stream.clone(), backlog.clone());
    let sender = thread::Builder::new()
        .name("ubiq-tunnel-tls-send".into())
        .spawn(move || send_loop(&send_stream, out_rx, &send_backlog))
        .map_err(|e| e.to_string())?;

    let mut reader = local.try_clone().map_err(|e| e.to_string())?;
    let (feed_mux, feed_tx, feed_counters, feed_backlog) =
        (mux.clone(), tx.clone(), counters.clone(), backlog.clone());
    thread::Builder::new()
        .name("ubiq-tunnel-tls-up".into())
        .spawn(move || {
            let mut buf = vec![0u8; TUNNEL_CHUNK_MAX];
            let mut out = Vec::new();
            loop {
                if !feed_backlog.wait_room() {
                    return;
                }
                feed_mux.rate_wait();
                let event = match reader.read(&mut buf) {
                    Ok(0) => Event::LocalEof,
                    Ok(n) => {
                        feed_mux.rate_debit(n);
                        feed_counters
                            .up
                            .fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
                        out.clear();
                        match rewriter.feed(&buf[..n], &mut out) {
                            Ok(()) => Event::Local(std::mem::take(&mut out)),
                            // A refusal is answered by the response side once it is idle; the
                            // upstream is told the requests are over, as at an EOF.
                            Err(e) if crate::http::is_refusal(&e) => {
                                feed_counters.error(e.to_string());
                                Event::LocalEof
                            }
                            Err(e) => Event::LocalError(e.to_string()),
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => Event::LocalError(format!("the local socket failed: {e}")),
                };
                let last = !matches!(event, Event::Local(_));
                if feed_tx.send(event).is_err() || last {
                    return;
                }
            }
        })
        .map_err(|e| e.to_string())?;

    let feed_stream = stream.clone();
    thread::Builder::new()
        .name("ubiq-tunnel-tls-down".into())
        .spawn(move || {
            loop {
                let event = match feed_stream.recv() {
                    Recv::Data(bytes) => Event::Remote(bytes),
                    Recv::Eof => Event::RemoteEof,
                    Recv::Reset(e) => Event::RemoteReset(e),
                };
                let last = !matches!(event, Event::Remote(_));
                if tx.send(event).is_err() || last {
                    return;
                }
            }
        })
        .map_err(|e| e.to_string())?;

    let mut local_done = false;
    let mut remote_done = false;
    let mut plain = vec![0u8; TUNNEL_CHUNK_MAX];
    let mut rewritten = Vec::new();
    // The response rewriter's last word once the server is done, before the local half-close.
    let mut ended = false;
    let mut end_response = |local: &mut TcpStream, response: &mut ResponseRewriter| {
        if !ended {
            ended = true;
            let mut rest = Vec::new();
            response.finish(&mut rest);
            let _ = local.write_all(&rest);
        }
        let _ = local.shutdown(Shutdown::Write);
    };
    loop {
        flush(&mut conn, &out_tx, &backlog)?;
        if local_done && remote_done {
            // Everything handed over: wait for the sender to put it on the stream.
            drop(out_tx);
            return match sender.join() {
                Ok(result) => result.map_err(|e| e.to_string()),
                Err(_) => Err("the TLS sender panicked".into()),
            };
        }
        let Ok(event) = rx.recv() else {
            return Err("the tunnel stream ended".into());
        };
        match event {
            Event::Local(bytes) => conn.writer().write_all(&bytes).map_err(|e| e.to_string())?,
            Event::LocalEof => {
                // TLS's own half-close, then the stream's: what a plain socket's EOF is.
                local_done = true;
                conn.send_close_notify();
                flush(&mut conn, &out_tx, &backlog)?;
                out_tx
                    .send(Outbound::CloseWrite)
                    .map_err(|_| "the tunnel stream ended".to_string())?;
            }
            Event::LocalError(e) => return Err(e),
            Event::Remote(bytes) => {
                let mut rest = &bytes[..];
                while !rest.is_empty() {
                    conn.read_tls(&mut rest).map_err(|e| format!("TLS: {e}"))?;
                    conn.process_new_packets()
                        .map_err(|e| format!("TLS with the upstream: {e}"))?;
                    loop {
                        match conn.reader().read(&mut plain) {
                            Ok(0) => {
                                // The server's close_notify: no more response.
                                end_response(local, &mut response);
                                break;
                            }
                            Ok(n) => {
                                rewritten.clear();
                                response.feed(&plain[..n], &mut rewritten);
                                local
                                    .write_all(&rewritten)
                                    .map_err(|e| format!("the local socket failed: {e}"))?;
                                counters
                                    .down
                                    .fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
                            }
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                            Err(e) => return Err(format!("TLS with the upstream: {e}")),
                        }
                    }
                }
                stream.ack(bytes.len());
                if let Some(answer) = response.farewell() {
                    let _ = local.write_all(&answer);
                    let _ = local.shutdown(Shutdown::Both);
                    stream.reset(None);
                    return Ok(());
                }
            }
            Event::RemoteEof => {
                remote_done = true;
                end_response(local, &mut response);
            }
            Event::RemoteReset(e) => {
                return Err(e.unwrap_or_else(|| "the far end reset the stream".into()));
            }
        }
    }
}
