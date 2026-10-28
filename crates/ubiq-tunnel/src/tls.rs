//! `upstream_tls`: the window speaks TLS to the far server itself, so plain `http://localhost`
//! reaches an `https://` upstream. The TLS session ends here, not on the drone — the drone pipes
//! ciphertext and never holds a key.
//!
//! A rustls client connection cannot be read on one thread and written on another, so one thread
//! owns it and two feeders — the local socket, the tunnel stream — post to it.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, OnceLock};
use std::thread;

use rustls::ClientConnection;
use rustls::pki_types::ServerName;
use ubiq_proto::tunnel::TUNNEL_CHUNK_MAX;

use crate::http::HostRewriter;
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

/// [`crate::pipe::pipe`] with a TLS client between the local socket and the stream. Blocks.
pub(crate) fn pipe_tls(
    mux: &Mux,
    mut local: TcpStream,
    stream: TunnelStream,
    rewriter: HostRewriter,
    config: Arc<rustls::ClientConfig>,
    server: ServerName<'static>,
    counters: Arc<Counters>,
) {
    let stream = Arc::new(stream);
    let result = run(
        mux, &mut local, &stream, rewriter, config, server, &counters,
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
    mut rewriter: HostRewriter,
    config: Arc<rustls::ClientConfig>,
    server: ServerName<'static>,
    counters: &Arc<Counters>,
) -> Result<(), String> {
    let mut conn = ClientConnection::new(config, server).map_err(|e| format!("TLS: {e}"))?;
    // The local feeder is already bounded by its queue; rustls' own cap would refuse early data.
    conn.set_buffer_limit(None);
    let (tx, rx) = flume::bounded::<Event>(8);

    let mut reader = local.try_clone().map_err(|e| e.to_string())?;
    let (feed_mux, feed_tx, feed_counters) = (mux.clone(), tx.clone(), counters.clone());
    thread::Builder::new()
        .name("ubiq-tunnel-tls-up".into())
        .spawn(move || {
            let mut buf = vec![0u8; TUNNEL_CHUNK_MAX];
            let mut out = Vec::new();
            loop {
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
    let mut records = Vec::new();
    let mut plain = vec![0u8; TUNNEL_CHUNK_MAX];
    loop {
        records.clear();
        while conn.wants_write() {
            conn.write_tls(&mut records)
                .map_err(|e| format!("TLS: {e}"))?;
        }
        if !records.is_empty() {
            stream.send(&records).map_err(|e| e.to_string())?;
        }
        if local_done && remote_done {
            return Ok(());
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
                records.clear();
                while conn.wants_write() {
                    conn.write_tls(&mut records)
                        .map_err(|e| format!("TLS: {e}"))?;
                }
                stream.send(&records).map_err(|e| e.to_string())?;
                stream.close_write();
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
                                let _ = local.shutdown(Shutdown::Write);
                                break;
                            }
                            Ok(n) => {
                                local
                                    .write_all(&plain[..n])
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
            }
            Event::RemoteEof => {
                remote_done = true;
                let _ = local.shutdown(Shutdown::Write);
            }
            Event::RemoteReset(e) => {
                return Err(e.unwrap_or_else(|| "the far end reset the stream".into()));
            }
        }
    }
}
