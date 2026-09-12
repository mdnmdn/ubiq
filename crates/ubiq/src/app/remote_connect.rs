//! Reach a remote host from the UI, and the client half of the wire once it does.
//!
//! **Why this lives here and not as its own file elsewhere in `app/`.** The transport section
//! below (`dial`, the handshake, the two pump threads) has exactly one production caller family:
//! the connect modal, the reconnect loop and the manager panel's test button. The doc comments
//! below are the seam instead of a second module.
//!
//! **The transport is deliberately dumb.** Exactly like `ubiq_host::remote`'s listener, this knows
//! nothing about any message family — it is attach, pump frames onto a `Client`, pump the
//! `Client`'s outbox onto the socket, and stop when either side does. Read `ubiq_host::remote`
//! alongside this: the two are mirror images of the same handshake, one binding and one dialing.
//!
//! **Never on the GPUI thread.** Every step in [`dial`] is a blocking syscall — a DNS lookup, a
//! TCP connect, a TLS handshake, a byte-at-a-time header read. [`AppState::try_connect_remote`]
//! runs it inside `cx.background_spawn`, exactly the way `chat.rs` farms out a diagram render,
//! and reads the answer back through `cx.spawn` updating the entity. A modal that called this
//! straight from a click handler would freeze the whole window for as long as a dead address
//! takes to time out.
//!
//! **Reconnects replace nothing the user can see.** A dropped socket closes the panes it ran —
//! the far side reaps them on `Gone`, so there is nothing to reattach to — and a successful
//! redial re-registers the same saved entry under a new `HostId`, re-asks `ListProjects` and
//! clears the failed mark. The saved entry's keychain token is what makes that possible
//! without asking the user again.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use ubiq_proto::bus::{self, Client, FromClient};
use ubiq_proto::carrier::{Beat, Heartbeat};
use ubiq_proto::settings::RemoteScheme;
use ubiq_proto::wire;

use crate::state::remote::{AttemptId, RemoteConnectState, RemoteConnectStep, with_default_port};

use super::*;

// ── Transport ───────────────────────────────────────────────────────────────

/// How long a dial waits — for the TCP handshake, and separately for each line of the HTTP
/// response — before giving up. A wrong address must not hang the modal for however long the
/// platform's own connect timeout is (which can be minutes on some networks); this is Ubiq's own,
/// short ceiling instead.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(6);

/// A response header claiming more than this before the blank line that ends it is refused.
/// Mirrors `ubiq_host::remote::MAX_HEADER` — the same ceiling, on the other end of the same
/// handshake, for the same reason: an unbounded read here would let a broken or hostile peer make
/// this process buffer without limit.
const MAX_HEADER: usize = 8 * 1024;

/// How often the writer thread wakes to check whether the reader has given up, when there is
/// nothing queued to write. Mirrors `ubiq_host::remote::STOP_POLL`.
const STOP_POLL: Duration = Duration::from_millis(200);

/// Why a dial did not end in an attached `Client`, worded for the modal to show directly.
#[derive(Clone, Debug)]
pub enum ConnectFailure {
    /// The address did not resolve, or nothing answered the TCP handshake before
    /// [`CONNECT_TIMEOUT`].
    Unreachable(String),
    /// The host answered `401`: the token was wrong.
    TokenRejected,
    /// The host answered, but not with the `101` this handshake needs.
    Refused(String),
    /// The TLS handshake failed, or the certificate was refused. The string names which —
    /// an untrusted certificate says so explicitly, because the fix is the trust checkbox.
    Tls(String),
    /// The socket failed, or timed out, somewhere after the connection was made.
    Io(String),
    /// The address or the token carried a control character, so no request was sent.
    Untypable(&'static str),
}

/// Whether a value may be interpolated into the request text.
///
/// A connection string is pasted from somewhere else — that is the whole point of it — so its
/// halves are untrusted text, not something the user typed on purpose. A carriage return or a
/// newline in either one would end the request line and let whatever followed be read as further
/// headers or a second request, which is request smuggling with the user's own hand on the paste.
/// A real token is base64url and a real address is a host and a port, so nothing legitimate is
/// turned away by refusing every control character.
fn is_typable(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(|c| c.is_control())
}

impl std::fmt::Display for ConnectFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectFailure::Unreachable(reason) => {
                write!(f, "could not reach that address: {reason}")
            }
            ConnectFailure::TokenRejected => write!(f, "that host rejected the token"),
            ConnectFailure::Refused(reason) => {
                write!(f, "that host did not answer as a Ubiq host: {reason}")
            }
            ConnectFailure::Tls(reason) => {
                write!(f, "the secure connection failed: {reason}")
            }
            ConnectFailure::Io(reason) => write!(f, "the connection failed: {reason}"),
            ConnectFailure::Untypable(part) => {
                write!(
                    f,
                    "that {part} contains a character an address cannot carry"
                )
            }
        }
    }
}

/// The `GET /attach…` request text, factored out so a test can pin down its exact shape with no
/// socket at all.
///
/// The token rides the query string, matching `ubiq_host::remote::read_request`'s own parsing —
/// see that function's `path.strip_prefix("/attach?")` and the `token=` lookup right after it.
fn attach_request(address: &str, token: &str) -> String {
    format!(
        "GET /attach?token={token} HTTP/1.1\r\n\
         Host: {address}\r\n\
         Upgrade: ubiq\r\n\
         Connection: Upgrade\r\n\
         \r\n"
    )
}

/// Pull the numeric status out of a line like `HTTP/1.1 101 Switching Protocols`.
fn parse_status_code(status_line: &str) -> Option<u16> {
    status_line.split_whitespace().nth(1)?.parse().ok()
}

/// Read one line, one byte at a time, refusing to grow past `budget`. `Ok(None)` is "ran out of
/// budget before a newline"; `Ok(Some(0))` is a clean EOF with nothing read at all.
///
/// One byte per syscall, deliberately not a `BufReader`: exactly the trade `ubiq_host::remote`'s
/// own `read_capped_line` makes, and for the same reason — a buffered reader can pull well past
/// the header block's end in one underlying read, and everything past the blank line that ends
/// the response is the first wire frame once the upgrade lands. Swallowing any of it into a buffer
/// this function is about to drop would desync the two ends from the very first message.
///
/// Generic over the stream so the same handshake runs over plaintext TCP and over TLS without a
/// second copy of it.
fn read_capped_line(
    stream: &mut dyn Read,
    line: &mut String,
    budget: usize,
) -> io::Result<Option<usize>> {
    let mut byte = [0u8; 1];
    let mut read = 0usize;
    loop {
        if read >= budget {
            return Ok(None);
        }
        match stream.read(&mut byte) {
            Ok(0) => return Ok(Some(read)),
            Ok(_) => {
                read += 1;
                line.push(byte[0] as char);
                if byte[0] == b'\n' {
                    return Ok(Some(read));
                }
            }
            Err(error) => return Err(error),
        }
    }
}

/// Read the response's status line and headers up to the blank line that ends them, and no
/// further — see [`read_capped_line`]'s comment on why stopping exactly there matters.
fn read_response_status(stream: &mut dyn Read) -> Result<u16, ConnectFailure> {
    let mut total = 0usize;
    let mut status_line: Option<String> = None;
    loop {
        let mut line = String::new();
        let read = read_capped_line(stream, &mut line, MAX_HEADER - total)
            .map_err(|error| ConnectFailure::Io(error.to_string()))?;
        match read {
            None => {
                return Err(ConnectFailure::Refused(
                    "response header too large".to_string(),
                ));
            }
            Some(0) => {
                return Err(ConnectFailure::Io(
                    "the connection closed during the handshake".to_string(),
                ));
            }
            Some(n) => total += n,
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if status_line.is_none() {
            status_line = Some(trimmed.to_string());
        }
        if trimmed.is_empty() {
            break;
        }
    }
    let status_line =
        status_line.ok_or_else(|| ConnectFailure::Refused("empty response".to_string()))?;
    parse_status_code(&status_line).ok_or(ConnectFailure::Refused(status_line))
}

/// What a dial needs. The modal owns these as fields and inputs; the reconnect loop and the
/// test button build them from a saved entry.
#[derive(Clone, Debug)]
pub struct DialParams {
    /// `host:port`, default port already applied.
    pub address: String,
    pub token: String,
    pub scheme: RemoteScheme,
    pub trust_insecure: bool,
}

/// Dial one remote host over plaintext and, on success, hand back the `Client` a `Bus`
/// registers. The scheme-carrying [`dial_with`] below is what the modal and the loop use; this
/// stays as the plaintext shorthand its doc comment always described.
#[allow(dead_code)]
fn dial(address: &str, token: &str) -> Result<Client, ConnectFailure> {
    dial_with(&DialParams {
        address: address.to_string(),
        token: token.to_string(),
        scheme: RemoteScheme::Http,
        trust_insecure: false,
    })
}

fn dial_with(params: &DialParams) -> Result<Client, ConnectFailure> {
    let session = dial_raw(params)?;
    let (client, detached) = bus::detached();
    spawn_pump(session.reader, session.writer, session.closer, detached);
    Ok(client)
}

/// One side of [`dial_raw`]'s answer: the two stream halves plus a TCP clone used only to
/// `shutdown` the kernel socket when the writer gives up.
struct RawSession {
    reader: Box<dyn Socket>,
    writer: Box<dyn Socket>,
    closer: TcpStream,
}

/// Dial and run the HTTP upgrade, handing back the two stream halves with no pumps behind them.
///
/// What `dial_with` attaches to the bus, and what the test button drives by hand: one `Stats`
/// poll down the writer, frames off the reader until `Stats` (skipping the unsolicited
/// `HostInfo` greeting), then dropped. Split out so the test path never registers a `Client`
/// nobody would drain.
fn dial_raw(params: &DialParams) -> Result<RawSession, ConnectFailure> {
    // Before the socket, not after: a request that must not be sent is one this never builds.
    if !is_typable(&params.address) {
        return Err(ConnectFailure::Untypable("address"));
    }
    if !is_typable(&params.token) {
        return Err(ConnectFailure::Untypable("token"));
    }

    let socket_addr = params
        .address
        .to_socket_addrs()
        .map_err(|error| ConnectFailure::Unreachable(error.to_string()))?
        .next()
        .ok_or_else(|| {
            ConnectFailure::Unreachable(format!("no address found for {}", params.address))
        })?;

    let stream = TcpStream::connect_timeout(&socket_addr, CONNECT_TIMEOUT)
        .map_err(|error| ConnectFailure::Unreachable(error.to_string()))?;
    stream
        .set_nodelay(true)
        .map_err(|error| ConnectFailure::Io(error.to_string()))?;
    // Bounds the handshake itself: a peer that completes the TCP handshake but never answers the
    // HTTP one (a firewall swallowing the request, say) would otherwise hang this thread forever.
    // Cleared once the upgrade lands, below, so a session that runs for hours is never mistaken
    // for a stalled read.
    stream
        .set_read_timeout(Some(CONNECT_TIMEOUT))
        .map_err(|error| ConnectFailure::Io(error.to_string()))?;

    // From here the two schemes share everything but the stream: TLS wraps the socket first,
    // then the same HTTP upgrade runs over (plaintext) or inside (TLS) it.
    match params.scheme {
        RemoteScheme::Http => {
            let mut stream = Box::new(stream);
            stream
                .write_all(attach_request(&params.address, &params.token).as_bytes())
                .map_err(|error| ConnectFailure::Io(error.to_string()))?;
            match read_response_status(&mut stream)? {
                101 => {}
                401 => return Err(ConnectFailure::TokenRejected),
                other => return Err(ConnectFailure::Refused(format!("host answered {other}"))),
            }
            // The deadline was the handshake's, not the session's: a client that says nothing
            // for an hour is an idle window, not a stalled peer.
            stream
                .set_read_timeout(None)
                .map_err(|error| ConnectFailure::Io(error.to_string()))?;
            let closer = stream
                .try_clone()
                .map_err(|error| ConnectFailure::Io(error.to_string()))?;
            let reader = stream
                .try_clone()
                .map_err(|error| ConnectFailure::Io(error.to_string()))?;
            Ok(RawSession {
                reader: Box::new(reader) as Box<dyn Socket>,
                writer: stream as Box<dyn Socket>,
                closer,
            })
        }
        RemoteScheme::Https => {
            let mut pair = tls_handshake(stream, &params.address, params.trust_insecure)?;
            rustls::Stream::new(&mut pair.conn, &mut pair.sock)
                .write_all(attach_request(&params.address, &params.token).as_bytes())
                .map_err(|error| ConnectFailure::Io(error.to_string()))?;
            let status = {
                let mut io = rustls::Stream::new(&mut pair.conn, &mut pair.sock);
                read_response_status(&mut io)?
            };
            match status {
                101 => {}
                401 => return Err(ConnectFailure::TokenRejected),
                other => return Err(ConnectFailure::Refused(format!("host answered {other}"))),
            }
            let _ = pair.sock.set_read_timeout(None);
            let closer = pair
                .sock
                .try_clone()
                .map_err(|error| ConnectFailure::Io(error.to_string()))?;
            let shared = SharedTls(Arc::new(Mutex::new(pair)));
            Ok(RawSession {
                reader: Box::new(shared.clone()),
                writer: Box::new(shared),
                closer,
            })
        }
    }
}

/// A socket half the pumps can own: anything readable, writable and sendable across threads.
///
/// Plaintext dials hand over a `TcpStream` and its `try_clone`; TLS dials hand over two handles
/// to the same shared session (rustls has no `try_clone`, so the session is locked per call).
trait Socket: Read + Write + Send + 'static {}

impl<T: Read + Write + Send + 'static> Socket for T {}

/// The TLS half of a dial: one session, shared by the reader and writer pumps behind a lock.
///
/// rustls has no `try_clone`, so the two directions hold handles to the same session and take
/// the lock per call — short enough that a frame write never blocks a frame read for long, and
/// the only alternative is a second TLS session the host would read as a second client.
#[derive(Clone)]
struct SharedTls(Arc<Mutex<TlsPair>>);

struct TlsPair {
    conn: rustls::ClientConnection,
    sock: TcpStream,
}

fn lock_pair(pair: &Mutex<TlsPair>) -> std::sync::MutexGuard<'_, TlsPair> {
    pair.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Read for SharedTls {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut pair = lock_pair(&self.0);
        let TlsPair { conn, sock } = &mut *pair;
        rustls::Stream::new(conn, sock).read(buf)
    }
}

impl Write for SharedTls {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut pair = lock_pair(&self.0);
        let TlsPair { conn, sock } = &mut *pair;
        rustls::Stream::new(conn, sock).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut pair = lock_pair(&self.0);
        let TlsPair { conn, sock } = &mut *pair;
        rustls::Stream::new(conn, sock).flush()
    }
}

/// Wrap a connected socket in TLS and run the handshake to completion.
///
/// Blocking, like everything else in this file — the caller is already off the GPUI thread.
/// `address` is `host:port`; the port is stripped before the server name is derived from it.
fn tls_handshake(
    sock: TcpStream,
    address: &str,
    trust_insecure: bool,
) -> Result<TlsPair, ConnectFailure> {
    use rustls::pki_types::ServerName;

    let host = address
        .rsplit_once(':')
        .map(|(host, _)| host)
        .unwrap_or(address)
        .trim()
        .trim_matches(['[', ']']);
    if host.is_empty() {
        return Err(ConnectFailure::Unreachable("empty host".to_string()));
    }
    let server_name: ServerName<'static> = if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        ServerName::IpAddress(ip.into())
    } else {
        ServerName::try_from(host.to_string())
            .map_err(|_| ConnectFailure::Unreachable(format!("{host} is not a valid host name")))?
    };

    let config = tls_config(trust_insecure);
    let mut conn = rustls::ClientConnection::new(config, server_name)
        .map_err(|error| ConnectFailure::Tls(error.to_string()))?;
    let mut sock = sock;
    while conn.is_handshaking() {
        conn.complete_io(&mut sock)
            .map_err(|error| ConnectFailure::Tls(error.to_string()))?;
    }
    Ok(TlsPair { conn, sock })
}

/// The crypto provider, named rather than inherited — the host's `connectors::tls` names `ring`
/// for the same reason, and two halves of one handshake must not disagree about it.
fn tls_provider() -> Arc<rustls::crypto::CryptoProvider> {
    static PROVIDER: OnceLock<Arc<rustls::crypto::CryptoProvider>> = OnceLock::new();
    PROVIDER
        .get_or_init(|| Arc::new(rustls::crypto::ring::default_provider()))
        .clone()
}

/// The trust anchors: the platform's store first, the compiled-in public roots as the floor.
/// Same order as the host's own verifier — an internal CA works, and a machine whose store
/// cannot be read still reaches a public host.
fn tls_roots() -> Arc<rustls::RootCertStore> {
    static ROOTS: OnceLock<Arc<rustls::RootCertStore>> = OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let mut store = rustls::RootCertStore::empty();
            let native = rustls_native_certs::load_native_certs();
            for error in &native.errors {
                tracing::debug!("a platform trust root was not read: {error}");
            }
            store.add_parsable_certificates(native.certs);
            store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(store)
        })
        .clone()
}

fn tls_config(trust_insecure: bool) -> Arc<rustls::ClientConfig> {
    static CHAINED: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    static INSECURE: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    let slot = if trust_insecure { &INSECURE } else { &CHAINED };
    slot.get_or_init(|| {
        // Infallible with `ring` compiled in: the only failure is no usable cipher suite, and
        // the provider names its own.
        let builder = rustls::ClientConfig::builder_with_provider(tls_provider())
            .with_safe_default_protocol_versions()
            .expect("the TLS protocol versions");
        if trust_insecure {
            Arc::new(
                builder
                    .dangerous()
                    .with_custom_certificate_verifier(TrustAny::new())
                    .with_no_client_auth(),
            )
        } else {
            Arc::new(
                builder
                    .with_root_certificates(tls_roots())
                    .with_no_client_auth(),
            )
        }
    })
    .clone()
}

/// The verifier behind the trust checkbox: the chain is accepted without asking the
/// platform, but handshake signatures still go through the real verifier.
///
/// There is deliberately no pinning here — a fingerprint the user vouched for once would have
/// to live beside the saved entry, and silently re-accepting it afterwards is a second trust
/// decision this round does not take. The manager panel shows the flag so the choice stays
/// visible, and unchecking it returns to the platform's rules.
#[derive(Debug)]
struct TrustAny {
    inner: Arc<rustls::client::WebPkiServerVerifier>,
}

impl TrustAny {
    fn new() -> Arc<Self> {
        let inner = rustls::client::WebPkiServerVerifier::builder_with_provider(
            tls_roots(),
            tls_provider(),
        )
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
        _server_name: &rustls::pki_types::ServerName<'_>,
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

/// Shuttle frames between the socket and a [`bus::Detached`] until either side gives up.
///
/// The shape is `ubiq_host::remote::pump`'s, mirrored: there, one `Client` came from a `Hub` and
/// the socket was accepted; here, the `Client` is [`bus::detached`]'s and the socket was dialed.
/// Everything past that — own each direction on its own thread, poll `said()` so the writer
/// notices a dead reader without a blocking `recv` masking it, drop the halves from whichever
/// side notices first — is identical for the same reason: a session, once the handshake behind
/// it is done, does not care which end opened the connection.
fn spawn_pump(
    reader_stream: Box<dyn Socket>,
    writer_stream: Box<dyn Socket>,
    closer: TcpStream,
    detached: bus::Detached,
) {
    // Flume's `Sender`/`Receiver` clone by sharing the same queue, so the two threads below can
    // each own a handle with no `Arc` and no need to keep `detached` itself alive.
    let said = detached.said().clone();
    let deliver = detached.deliver().clone();

    let mut reader_stream = reader_stream;
    let mut writer_stream = writer_stream;

    // The interface's half of the heartbeat, and it is the same half: both ends of a carrier ping
    // and both answer, because either end can be the one that goes quiet. The rules are
    // `ubiq_proto::carrier`'s so this pump and `ubiq_host::carrier::pump` cannot drift — the
    // duplicated *threads* are forced by the crate boundary (`crates/ubiq` does not depend on
    // `crates/ubiq-host`), the duplicated rules would not be.
    let beat = Arc::new(Mutex::new(Heartbeat::new()));
    // A `Pong` the reader owes, on its way to the thread that owns the write half. It never
    // reaches `deliver`, and so never reaches `AppState`: a heartbeat is transport.
    let (pongs, pongs_out) = flume::unbounded();

    let writer_said = said;
    let writer_beat = beat.clone();
    thread::Builder::new()
        .name("ubiq-remote-client-writer".to_string())
        .spawn(move || {
            loop {
                // Checked before the wait, and so also after every frame written: a busy outbound
                // direction with a dead inbound one never reaches the timeout arm below.
                if writer_beat.lock().expect("the heartbeat").is_gone() {
                    tracing::info!("the remote host stopped answering: ending the session");
                    break;
                }
                let picked = flume::Selector::new()
                    .recv(&pongs_out, |taken| taken.ok().map(Outbound::Pong))
                    .recv(&writer_said, |taken| taken.ok().map(Outbound::Said))
                    .wait_timeout(STOP_POLL);
                match picked {
                    Ok(Some(Outbound::Pong(pong))) => {
                        if wire::write_frame(&mut writer_stream, &pong).is_err() {
                            break;
                        }
                    }
                    Ok(Some(Outbound::Said(FromClient::Said { message, .. }))) => {
                        if wire::write_frame(&mut writer_stream, &message).is_err() {
                            break;
                        }
                    }
                    // `Connected` never fires for a detached client — only `Hub::connect` sends
                    // it — and `Gone` is the local `Client` being dropped, an intentional
                    // teardown rather than a failure to report.
                    Ok(Some(Outbound::Said(FromClient::Connected(_)))) => {}
                    Ok(Some(Outbound::Said(FromClient::Gone(_)))) | Ok(None) => break,
                    Err(flume::select::SelectError::Timeout) => {
                        let owed = writer_beat.lock().expect("the heartbeat").due();
                        if let Some(ping) = owed
                            && wire::write_frame(&mut writer_stream, &ping).is_err()
                        {
                            break;
                        }
                    }
                }
            }
            // Shutdown the kernel socket while a handle still exists. Dropping a TLS
            // `SharedTls` or one TCP clone does not send EOF while the reader holds the
            // other half, so the host would never see `Gone` and would not reap panes.
            let _ = closer.shutdown(Shutdown::Both);
            drop(writer_stream);
        })
        .expect("the remote client's writer thread");

    thread::Builder::new()
        .name("ubiq-remote-client-reader".to_string())
        .spawn(move || {
            while let Ok(message) = wire::read_frame(&mut reader_stream) {
                let verdict = beat.lock().expect("the heartbeat").inbound(message);
                match verdict {
                    Beat::Deliver(message) => {
                        if deliver.send(message).is_err() {
                            break;
                        }
                    }
                    Beat::Answer(pong) => {
                        if pongs.send(pong).is_err() {
                            break;
                        }
                    }
                    Beat::Swallow => {}
                }
            }
            drop(reader_stream);
        })
        .expect("the remote client's reader thread");
}

/// What the writer thread picked up: a heartbeat answer it owes, or something the window said.
enum Outbound {
    Pong(Message),
    Said(FromClient),
}

// ── The modal ───────────────────────────────────────────────────────────────

impl AppState {
    /// Raise the connect modal, with both fields empty.
    pub fn open_remote_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_remote_connect_inputs(window, cx);
        self.workbench.remote_connect = Some(RemoteConnectState::default());
        cx.notify();
    }

    /// Raise the connect modal for a saved-but-unattached host, with the address filled in, the
    /// scheme and trust picks following the saved entry, and the token filled in when the
    /// keychain still holds it — left for the user to type when it does not.
    ///
    /// Reuses [`Self::open_remote_connect`]'s modal and [`Self::try_connect_remote`]'s dial
    /// rather than a second path: a saved host is just this same flow with most fields already
    /// known.
    #[allow(clippy::too_many_arguments)]
    pub fn reconnect_saved_host(
        &mut self,
        save_id: String,
        name: String,
        address: String,
        scheme: RemoteScheme,
        trust_insecure: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_remote_connect_inputs(window, cx);
        self.remote_address_input.update(cx, |state, cx| {
            state.set_value(&address, window, cx);
        });
        if let Some(token) = host_secrets::load_token(&host_secrets::key_for(&save_id, &address)) {
            self.remote_token_input.update(cx, |state, cx| {
                state.set_value(&token, window, cx);
            });
        }
        self.workbench.remote_connect = Some(RemoteConnectState {
            step: RemoteConnectStep::Editing,
            saved_name: Some(name),
            save_id,
            scheme,
            trust_insecure,
        });
        cx.notify();
    }

    /// Flip the connect modal's scheme pick. The address field keeps whatever it says — scheme
    /// and address are separate choices, and a paste carrying a scheme sets this itself.
    pub fn set_remote_scheme(&mut self, scheme: RemoteScheme, cx: &mut Context<Self>) {
        if let Some(state) = &mut self.workbench.remote_connect {
            state.scheme = scheme;
        }
        cx.notify();
    }

    /// Flip the connect modal's trust checkbox. Only meaningful for `https`; kept on the state
    /// regardless so a toggle-then-scheme ordering loses nothing.
    pub fn set_remote_trust(&mut self, trust_insecure: bool, cx: &mut Context<Self>) {
        if let Some(state) = &mut self.workbench.remote_connect {
            state.trust_insecure = trust_insecure;
        }
        cx.notify();
    }

    /// Close the modal. A dial still running in the background is left to finish — it holds no
    /// reference back to this state — and its answer is discarded on arrival because nothing
    /// still names its `AttemptId`. See [`Self::land_remote_connect`].
    pub fn cancel_remote_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workbench.remote_connect = None;
        self.clear_remote_connect_inputs(window, cx);
        cx.notify();
    }

    /// Back to editing after a failure, with both fields left as they were: a wrong address is
    /// corrected by fixing it, not by retyping everything.
    pub fn retry_remote_connect(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = &mut self.workbench.remote_connect {
            state.step = RemoteConnectStep::Editing;
        }
        cx.notify();
    }

    fn clear_remote_connect_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [&self.remote_address_input, &self.remote_token_input] {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
    }

    /// The address field changed. A paste of a whole connection string fills the token field
    /// alongside it, follows a pasted scheme, and rewrites the address field down to just the
    /// address — the one paste a user reaching for this modal actually has in their clipboard,
    /// made to work without asking them to split it themselves first.
    pub(super) fn apply_remote_address_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let typed = self.remote_address_input.read(cx).value().to_string();
        let parsed = crate::state::remote::parse_connection_string(&typed);
        if let Some(scheme) = parsed.scheme
            && let Some(state) = &mut self.workbench.remote_connect
        {
            state.scheme = scheme;
        }
        if parsed.address == typed.trim() && parsed.token.is_none() {
            // An ordinary keystroke, or a paste of a bare address: nothing to rewrite.
            cx.notify();
            return;
        }
        let Some(token) = parsed.token else {
            cx.notify();
            return;
        };
        self.remote_address_input.update(cx, |state, cx| {
            state.set_value(parsed.address, window, cx);
        });
        self.remote_token_input.update(cx, |state, cx| {
            state.set_value(token, window, cx);
        });
    }

    /// Start a dial. Reads both fields plus the scheme and trust picks, mints the [`AttemptId`]
    /// the answer must carry to be believed, and runs [`dial_with`] on the background executor so
    /// this call returns immediately — the modal redraws itself in the `Connecting` step while the
    /// socket work happens elsewhere.
    pub fn try_connect_remote(&mut self, cx: &mut Context<Self>) {
        let address = self
            .remote_address_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let token = self.remote_token_input.read(cx).value().trim().to_string();
        if address.is_empty() || token.is_empty() {
            return;
        }
        let modal = self.workbench.remote_connect.clone().unwrap_or_default();
        let params = DialParams {
            address: with_default_port(&address),
            token,
            scheme: modal.scheme,
            trust_insecure: modal.trust_insecure,
        };
        let save_id = modal.save_id.clone();
        let saved_name = modal.saved_name.clone();

        let attempt = AttemptId::generate();
        if let Some(state) = &mut self.workbench.remote_connect {
            state.step = RemoteConnectStep::Connecting { attempt };
        }
        cx.notify();

        let outcome = cx.background_spawn(async move { dial_with(&params) });
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let outcome = outcome.await;
            let _ = this.update(cx, |this, cx| {
                this.land_remote_connect(attempt, outcome, save_id, saved_name, cx)
            });
        })
        .detach();
    }

    /// A dial's answer, landing back on the GPUI thread.
    ///
    /// Discards an answer whose `attempt` is not the one `Connecting` is still showing — the user
    /// cancelled, retried, or closed the modal while this was in flight, and painting over
    /// whatever replaced it would be the same bug a stale `RepoQueryId` answer is in the clone
    /// flow.
    fn land_remote_connect(
        &mut self,
        attempt: AttemptId,
        outcome: Result<Client, ConnectFailure>,
        save_id: String,
        saved_name: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let showing = matches!(
            &self.workbench.remote_connect,
            Some(RemoteConnectState {
                step: RemoteConnectStep::Connecting { attempt: current },
                ..
            }) if *current == attempt
        );
        if !showing {
            // The user cancelled, retried, or started a second attempt while this one was still
            // in flight. A `Client` that arrives late is dropped rather than attached — dropping
            // it tells whatever it dialled that this window is gone, which is the right thing to
            // say to a host nobody asked to keep talking to any more.
            return;
        }

        let address = self
            .remote_address_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let modal = self.workbench.remote_connect.clone().unwrap_or_default();

        match outcome {
            Ok(client) => {
                // A reconnect keeps the name the host was saved under; a fresh dial has none yet,
                // so the address it just proved reachable at becomes its name — the same thing
                // `RemoteConn::label` already showed for it before there was a saved-hosts list.
                let label = saved_name.clone().unwrap_or_else(|| address.clone());
                // The token proved good, so it is the one the keychain keeps — a later reconnect
                // dials with this rather than asking again. Filed under the saved entry, minted
                // here when this dial was the entry's first.
                let saved = self.save_remote_host(
                    save_id,
                    label.clone(),
                    address.clone(),
                    modal.scheme,
                    modal.trust_insecure,
                    cx,
                );
                let token = self.remote_token_input.read(cx).value().trim().to_string();
                if !token.is_empty() {
                    host_secrets::save_token(&host_secrets::key_for(&saved, &address), &token);
                }
                self.attach_remote(
                    client,
                    label.clone(),
                    saved,
                    address.clone(),
                    modal.scheme,
                    cx,
                );
                self.workbench.settings.failed_hosts.remove(&address);
                if let Some(state) = &mut self.workbench.remote_connect {
                    state.step = RemoteConnectStep::Connected { label };
                }
            }
            Err(failure) => {
                // Recorded even for a first-time dial, harmlessly: `host_menu_rows` only ever
                // consults this set for an address that is also in `remote_hosts`, and this
                // address is not there until a dial to it succeeds.
                self.workbench.settings.failed_hosts.insert(address);
                if let Some(state) = &mut self.workbench.remote_connect {
                    state.step = RemoteConnectStep::Failed {
                        reason: failure.to_string(),
                    };
                }
            }
        }
        cx.notify();
    }

    /// Register a dialled connection on the bus and start draining it.
    ///
    /// Reuses [`Self::route_host`] rather than duplicating `boot.rs`'s router loop — a message
    /// arriving over this connection has to reach `receive` tagged with its `HostRef` exactly as a
    /// local one does, and that tagging is all a router task is.
    ///
    /// `label` is the saved host's own name for a reconnect, or the bare address for a fresh
    /// dial — see [`Self::land_remote_connect`], the only caller.
    fn attach_remote(
        &mut self,
        client: Client,
        label: String,
        save_id: String,
        address: String,
        scheme: RemoteScheme,
        cx: &mut Context<Self>,
    ) {
        let (host_id, from_host) = self
            .bus
            .register_remote(client, label, save_id, address, scheme);
        Self::route_host(HostRef::Remote(host_id), from_host, cx);
        // Nothing about a remote is known until it says so, and it says nothing unasked: the one
        // `ListProjects` boot.rs sends went to the local host, long before this connection
        // existed. Addressed rather than sent, because `active` still points wherever the user
        // left it — attaching a host is not choosing it.
        self.bus
            .send_to(HostRef::Remote(host_id), Message::ListProjects);
        // And its own readings, so the manager panel has facts without opening the Control
        // screen: the greeting carries the machine, the poll carries the moment.
        self.bus
            .send_to(HostRef::Remote(host_id), Message::ListStats);
    }

    // ── Reconnects ──────────────────────────────────────────────────────────

    /// The delay before the next try, from attempts since the drop. Doubles from two seconds
    /// and caps at a minute — a host that went away for lunch is found within a minute of its
    /// return without hammering one that is gone for the weekend.
    pub fn backoff_for(attempt: u32) -> Duration {
        Duration::from_secs(2u64.saturating_mul(1u64 << attempt.min(5)))
            .min(Duration::from_secs(60))
    }

    /// One saved entry by its keychain key, whatever names it now. A rename changes the name,
    /// never the key, so a reconnect scheduled before one still lands on the right entry.
    fn saved_entry(&self, key: &str) -> Option<ubiq_proto::settings::SavedRemoteHost> {
        self.workbench
            .settings
            .host
            .remote_hosts
            .iter()
            .find(|host| host_secrets::key_for(&host.id, &host.address) == key)
            .cloned()
    }

    /// A live connection dropped under the window — the socket failed, not the user. Its panes
    /// are closed the same way a manual disconnect closes them (the far side reaps them on
    /// `Gone`, so there is nothing to reattach to), and when the connection belonged to a saved
    /// entry the reconnect loop starts: the keychain token dials again without being asked.
    ///
    /// A one-off dial with no saved entry behind it keeps the old behaviour — a failed mark and
    /// a log line, and nothing scheduled.
    pub(super) fn remote_socket_lost(&mut self, id: HostId, cx: &mut Context<Self>) {
        let saved = self
            .bus
            .remote(id)
            .map(|remote| (remote.save_id.clone(), remote.address.clone()));
        let label = self.disconnect_host(id, cx);
        tracing::warn!("remote host {label} disconnected");
        let Some((save_id, address)) = saved else {
            return;
        };
        let key = host_secrets::key_for(&save_id, &address);
        if save_id.is_empty() && self.saved_entry(&key).is_none() {
            if let Some(address) = self.address_of_host(&label) {
                self.workbench.settings.failed_hosts.insert(address);
            }
            return;
        }
        self.workbench.settings.failed_hosts.insert(address.clone());
        // TrustAny accepts any certificate. Auto-reconnect would re-apply that without a
        // prompt, so a later MITM on this address would succeed. A click on Connect still
        // can, because that is the user saying so again.
        if self
            .saved_entry(&key)
            .is_some_and(|host| host.trust_insecure)
        {
            return;
        }
        self.start_reconnect(&key, cx);
    }

    /// Begin (or re-begin) the reconnect loop for a saved entry: bump the generation so a timer
    /// scheduled earlier aborts, reset the attempt count for a manual start, and dial at once
    /// rather than waiting out a backoff the user just asked to skip.
    pub fn start_reconnect(&mut self, key: &str, cx: &mut Context<Self>) {
        let generation = self
            .workbench
            .settings
            .reconnects
            .get(key)
            .map(|state| state.generation + 1)
            .unwrap_or(0);
        self.workbench.settings.reconnects.insert(
            key.to_string(),
            crate::state::settings::ReconnectState {
                attempt: 0,
                error: String::new(),
                generation,
            },
        );
        self.retry_reconnect(key.to_string(), generation, cx);
    }

    /// Stop the loop for a saved entry. Every scheduled wake checks the generation it was
    /// scheduled under and aborts on a mismatch — removing the entry stops all of them at once.
    pub fn stop_reconnect(&mut self, key: &str, cx: &mut Context<Self>) {
        self.workbench.settings.reconnects.remove(key);
        cx.notify();
    }

    /// One attempt, now. Reads the saved entry and its keychain token fresh every time, so an
    /// edit or a re-saved token between tries is honoured without stopping the loop.
    fn retry_reconnect(&mut self, key: String, generation: u64, cx: &mut Context<Self>) {
        let current = self.workbench.settings.reconnects.get(&key).cloned();
        let Some(state) = current else {
            return;
        };
        if state.generation != generation {
            return;
        }
        let Some(saved) = self.saved_entry(&key) else {
            self.workbench.settings.reconnects.remove(&key);
            cx.notify();
            return;
        };
        let address = saved.address.clone();
        let Some(token) = host_secrets::load_token(&key) else {
            // No secret, no loop: retrying without one is a dial that cannot succeed. The entry
            // stays with the reason on it, so the panel says what to do instead of spinning.
            if let Some(state) = self.workbench.settings.reconnects.get_mut(&key) {
                state.error = "no saved token — connect once from the panel to keep it".to_string();
            }
            cx.notify();
            return;
        };
        let params = DialParams {
            address: with_default_port(&address),
            token,
            scheme: saved.scheme,
            trust_insecure: saved.trust_insecure,
        };
        let label = saved.name.clone();
        let save_id = saved.id.clone();
        let outcome = cx.background_spawn(async move { dial_with(&params) });
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let outcome = outcome.await;
            let _ = this.update(cx, |this, cx| {
                this.land_reconnect(
                    key,
                    generation,
                    label,
                    save_id,
                    address,
                    saved.scheme,
                    outcome,
                    cx,
                )
            });
        })
        .detach();
    }

    /// A reconnect attempt's answer, landing back on the GPUI thread.
    #[allow(clippy::too_many_arguments)]
    fn land_reconnect(
        &mut self,
        key: String,
        generation: u64,
        label: String,
        save_id: String,
        address: String,
        scheme: RemoteScheme,
        outcome: Result<Client, ConnectFailure>,
        cx: &mut Context<Self>,
    ) {
        let current = self.workbench.settings.reconnects.get(&key).cloned();
        let Some(state) = current else {
            return;
        };
        if state.generation != generation {
            return;
        }
        match outcome {
            Ok(client) => {
                self.workbench.settings.reconnects.remove(&key);
                self.workbench.settings.failed_hosts.remove(&address);
                self.attach_remote(client, label, save_id, address, scheme, cx);
            }
            Err(failure) => {
                let attempt = state.attempt + 1;
                if let Some(state) = self.workbench.settings.reconnects.get_mut(&key) {
                    state.attempt = attempt;
                    state.error = failure.to_string();
                }
                let delay = Self::backoff_for(attempt);
                cx.notify();
                cx.spawn(async move |this: WeakEntity<Self>, cx| {
                    cx.background_executor().timer(delay).await;
                    let _ = this.update(cx, |this, cx| this.retry_reconnect(key, generation, cx));
                })
                .detach();
                return;
            }
        }
        cx.notify();
    }

    /// Dial a saved entry once and report, without attaching: the manager panel's test button.
    /// Reads the keychain token; without one the outcome says to connect once first, which is
    /// the flow that files it.
    pub fn test_saved_host(&mut self, key: String, cx: &mut Context<Self>) {
        let Some(saved) = self.saved_entry(&key) else {
            return;
        };
        self.workbench.remote_manager.test_started(key.clone());
        let address = saved.address.clone();
        let Some(token) = host_secrets::load_token(&key) else {
            self.workbench.remote_manager.test_finished(
                key,
                false,
                "no saved token — connect once to keep it".to_string(),
            );
            cx.notify();
            return;
        };
        let params = DialParams {
            address: with_default_port(&address),
            token,
            scheme: saved.scheme,
            trust_insecure: saved.trust_insecure,
        };
        let outcome = cx.background_spawn(async move {
            match dial_raw(&params) {
                Err(failure) => format!("unreachable: {failure}"),
                Ok(mut session) => {
                    let _ = session.closer.set_read_timeout(Some(CONNECT_TIMEOUT));
                    probe_stats(&mut session.reader, &mut session.writer)
                }
            }
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let report = outcome.await;
            let ok = report.starts_with("reachable");
            let _ = this.update(cx, |this, cx| {
                this.workbench.remote_manager.test_finished(key, ok, report);
                cx.notify();
            });
        })
        .detach();
    }
}

/// One `ListStats` poll: skip the unsolicited `HostInfo` greeting `Hub::connect` sends, then
/// take the first `Stats`. Anything else, or a quiet socket, is a failed probe.
fn probe_stats(reader: &mut Box<dyn Socket>, writer: &mut Box<dyn Socket>) -> String {
    if wire::write_frame(writer, &Message::ListStats).is_err() {
        return "connected, but the host would not answer".to_string();
    }
    loop {
        match wire::read_frame(reader) {
            Ok(message) => match classify_probe_frame(&message) {
                ProbeFrame::Skip => continue,
                ProbeFrame::Reachable(report) => return report,
                ProbeFrame::Odd => return "connected, but the host answered oddly".to_string(),
            },
            Err(_) => return "connected, but the host went quiet".to_string(),
        }
    }
}

#[derive(Debug, PartialEq)]
enum ProbeFrame {
    Skip,
    Reachable(String),
    Odd,
}

fn classify_probe_frame(message: &Message) -> ProbeFrame {
    match message {
        Message::HostInfo { .. } => ProbeFrame::Skip,
        Message::Stats { stats } => ProbeFrame::Reachable(format!(
            "reachable — {} session{}, {} agents live",
            stats.sessions_count,
            if stats.sessions_count == 1 { "" } else { "s" },
            stats.agents_live
        )),
        _ => ProbeFrame::Odd,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pinned down with no socket at all: this is exactly the request
    /// `ubiq_host::remote::read_request` parses — a `GET /attach?token=…` with an `Upgrade: ubiq`
    /// header — so a change here that drifted from that parser would otherwise only show up as a
    /// real handshake failing.
    /// A pasted connection string is untrusted text, and the request is built by interpolation.
    /// A carriage return in either half would close the request line and let the rest be read as
    /// headers of its own, so the dial has to refuse before it builds anything.
    #[test]
    fn a_control_character_is_refused_before_a_request_is_built() {
        assert!(!is_typable("host:7420\r\nX-Smuggled: 1"));
        assert!(!is_typable("tok\r\nGET /attach?token=stolen HTTP/1.1"));
        assert!(!is_typable("token\nwith-a-newline"));
        assert!(!is_typable(""));
        // What a real banner prints stays acceptable: base64url, and a host with a port.
        assert!(is_typable("aB3-_xyZ"));
        assert!(is_typable("192.168.1.5:7420"));
    }

    #[test]
    fn attach_request_matches_the_host_side_handshake() {
        let request = attach_request("192.168.1.5:7420", "aB3-_xyZ");
        assert!(request.starts_with("GET /attach?token=aB3-_xyZ HTTP/1.1\r\n"));
        assert!(request.contains("Upgrade: ubiq\r\n"));
        assert!(request.contains("Host: 192.168.1.5:7420\r\n"));
        assert!(request.ends_with("\r\n\r\n"));
    }

    #[test]
    fn status_code_is_pulled_out_of_the_status_line() {
        assert_eq!(
            parse_status_code("HTTP/1.1 101 Switching Protocols"),
            Some(101)
        );
        assert_eq!(parse_status_code("HTTP/1.1 401 Unauthorized"), Some(401));
    }

    #[test]
    fn a_status_line_with_no_recognisable_code_parses_to_none() {
        assert_eq!(parse_status_code("not a status line"), None);
        assert_eq!(parse_status_code(""), None);
    }

    #[test]
    fn a_hostinfo_greeting_is_not_a_failed_probe() {
        assert_eq!(
            classify_probe_frame(&Message::HostInfo {
                config_root: String::new(),
                is_default: false,
                hostname: None,
                os: None,
                arch: None,
                triplet: None,
                cpu_count: None,
                mem_total_bytes: None,
                shared_workarea: None,
            }),
            ProbeFrame::Skip
        );
        let stats = ubiq_proto::stats::HostStats {
            sessions_count: 1,
            agents_live: 2,
            ..ubiq_proto::stats::HostStats::default()
        };
        assert_eq!(
            classify_probe_frame(&Message::Stats { stats }),
            ProbeFrame::Reachable("reachable — 1 session, 2 agents live".to_string())
        );
        assert_eq!(
            classify_probe_frame(&Message::ListProjects),
            ProbeFrame::Odd
        );
    }
}
