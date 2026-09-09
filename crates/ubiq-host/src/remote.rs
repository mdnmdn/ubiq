//! The remote listener: lets a UI on another machine attach to this host over TCP.
//!
//! Everything this module does is transport. Per rule 1 in `_docs/tech/architecture.md`, the only
//! thing a connection needs from the bus is [`ubiq_proto::bus::Hub::connect`] — it mints a
//! `Client`, registers it in the routing table and tells the coordinator it exists. So a
//! connection here is nothing but: attach, pump frames off the socket onto the client, pump
//! messages off the client onto the socket, and drop the client when either side stops. No message
//! family is special-cased, and none should ever be added here.
//!
//! **Why `std::net::TcpListener` and not `tiny_http`.** `tiny_http` is used elsewhere in this
//! workspace for a one-shot loopback request (the OAuth redirect), but it cannot hand back the raw
//! socket after answering — it owns the connection for the life of one response. This listener has
//! to answer one HTTP-shaped request and then keep the same socket for the life of a session,
//! reading and writing raw length-prefixed frames on it. That needs the socket itself, not a
//! library built around request/response. Do not "simplify" this to `tiny_http` later — it cannot
//! do the upgrade this needs.
//!
//! **TLS is the same handshake inside a session.** `serve_tls` wraps each accepted socket in a
//! rustls server session first; everything past the handshake — the HTTP upgrade, the pumps —
//! runs over a [`Socket`] that does not know which it is. A UI dials `https` exactly when the
//! host was started with `--tls-cert`/`--tls-key`, and `http` otherwise.

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use rand::RngCore;

use ubiq_proto::bus::Hub;
use ubiq_proto::wire;

/// A header claiming more than this before the blank line that ends it is refused before any more
/// of it is read — an unauthenticated peer gets no chance to make this host buffer without bound.
const MAX_HEADER: usize = 8 * 1024;

/// How long a peer has to finish its handshake. The header block is read a byte at a time from an
/// unauthenticated socket, so without a deadline a peer that connects and then says nothing pins a
/// connection thread for the life of the process — the `MAX_HEADER` cap bounds the memory that
/// costs and nothing bounds the time. Cleared the moment the upgrade is written, because a session
/// is idle between frames by design.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// What starting the listener hands back: where it ended up bound, the token a client must
/// present to attach, and whether the socket speaks TLS.
pub struct Serving {
    pub addr: SocketAddr,
    pub token: String,
    pub tls: bool,
}

/// Start the listener on a thread of its own and return immediately.
///
/// `hub` is cloned into the accept loop; the caller keeps its own clone for local windows exactly
/// as it does today — this adds a second kind of attacher, not a different `Hub`.
pub fn serve(hub: Hub, bind: SocketAddr) -> io::Result<Serving> {
    let listener = TcpListener::bind(bind)?;
    let addr = listener.local_addr()?;
    let token = generate_token();
    let accept_token = token.clone();

    thread::Builder::new()
        .name("ubiq-remote-listen".to_string())
        .spawn(move || accept_loop(listener, hub, accept_token, None))
        .expect("the remote listener thread");

    Ok(Serving {
        addr,
        token,
        tls: false,
    })
}

/// Start the listener with TLS: every accepted socket runs its handshake before the HTTP
/// upgrade, and a plaintext dial against it reads nothing but TLS bytes.
///
/// `cert_pem` is the chain, leaf first; `key_pem` the private key — PEM, as `--tls-cert` and
/// `--tls-key` hand them over. A key that matches none of the parsed private-key shapes, or a
/// chain with no certificate in it, refuses here rather than after binding.
pub fn serve_tls(
    hub: Hub,
    bind: SocketAddr,
    cert_pem: &[u8],
    key_pem: &[u8],
) -> io::Result<Serving> {
    let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
        rustls_pemfile::certs(&mut &*cert_pem)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "could not read --tls-cert"))?;
    if certs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--tls-cert holds no certificate",
        ));
    }
    let key = rustls_pemfile::private_key(&mut &*key_pem)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "could not read --tls-key"))?
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "--tls-key holds no private key")
        })?;
    let config = rustls::ServerConfig::builder_with_provider(tls_provider())
        .with_safe_default_protocol_versions()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;

    let listener = TcpListener::bind(bind)?;
    let addr = listener.local_addr()?;
    let token = generate_token();
    let accept_token = token.clone();

    thread::Builder::new()
        .name("ubiq-remote-listen".to_string())
        .spawn(move || accept_loop(listener, hub, accept_token, Some(Arc::new(config))))
        .expect("the remote listener thread");

    Ok(Serving {
        addr,
        token,
        tls: true,
    })
}

/// The crypto provider, named rather than inherited — the connector family's `tls::provider`
/// names `ring` for the same reason, and two halves of one handshake must not disagree about it.
fn tls_provider() -> Arc<rustls::crypto::CryptoProvider> {
    use std::sync::OnceLock;
    static PROVIDER: OnceLock<Arc<rustls::crypto::CryptoProvider>> = OnceLock::new();
    PROVIDER
        .get_or_init(|| Arc::new(rustls::crypto::ring::default_provider()))
        .clone()
}

/// 256 bits, rendered URL-safe with no padding — long enough to paste into a connection string and
/// short enough to actually paste.
fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64_url_no_pad(&bytes)
}

/// A tiny base64url encoder so this module needs no new dependency for what is, in the end, one
/// call site. Standard base64url alphabet, no `=` padding.
fn base64_url_no_pad(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((bytes.len() * 4).div_ceil(3));
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 0x3f) as usize] as char);
        }
    }
    out
}

/// Compare two byte strings without early-returning on a length mismatch or the first differing
/// byte, so how long the comparison takes leaks nothing about how much of a guessed token was
/// right. A length check up front, or a `Vec::iter().eq()`, would both let a timing side-channel
/// narrow the token byte by byte.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let len_ok = a.len() == b.len();
    // Walk both up to the longer length so the loop body itself never depends on which is longer;
    // out-of-bounds reads are replaced with 0, which only ever costs a guaranteed mismatch.
    let n = a.len().max(b.len());
    let mut diff: u8 = if len_ok { 0 } else { 1 };
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= x ^ y;
    }
    diff == 0
}

/// A socket half the pumps can own: anything readable, writable and sendable across threads.
///
/// An accepted TCP socket hands over itself and its `try_clone`; a TLS session hands over two
/// handles to the same session behind a lock, because rustls has no `try_clone`.
trait Socket: Read + Write + Send + 'static {}

impl<T: Read + Write + Send + 'static> Socket for T {}

/// The TLS half of an accepted connection: one session, shared by the reader and writer pumps
/// behind a lock. Same shape as the dialer's `SharedTls`, mirrored — see that type for why the
/// lock is per call rather than per direction.
#[derive(Clone)]
struct SharedTls(Arc<Mutex<TlsPair>>);

struct TlsPair {
    conn: rustls::ServerConnection,
    sock: TcpStream,
}

impl Read for SharedTls {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut pair = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let TlsPair { conn, sock } = &mut *pair;
        rustls::Stream::new(conn, sock).read(buf)
    }
}

impl Write for SharedTls {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut pair = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let TlsPair { conn, sock } = &mut *pair;
        rustls::Stream::new(conn, sock).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut pair = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let TlsPair { conn, sock } = &mut *pair;
        rustls::Stream::new(conn, sock).flush()
    }
}

/// Accept connections until the listener itself fails (the process is going down). One hostile or
/// slow peer is confined to its own thread, per the loop body below, so it can never block this
/// one or the coordinator behind `hub`.
///
/// With a TLS config, each socket runs its handshake on its connection thread before anything is
/// read from it — a plaintext dial against a TLS listener stalls there until the handshake
/// deadline, then is dropped, having read nothing it could mistake for an answer.
fn accept_loop(
    listener: TcpListener,
    hub: Hub,
    token: String,
    tls: Option<Arc<rustls::ServerConfig>>,
) {
    for incoming in listener.incoming() {
        let stream = match incoming {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!("remote: accept failed: {error}");
                continue;
            }
        };
        let hub = hub.clone();
        let token = token.clone();
        let tls = tls.clone();
        thread::Builder::new()
            .name("ubiq-remote-conn".to_string())
            .spawn(move || handle_accepted(stream, hub, token, tls))
            .ok();
    }
}

/// One accepted socket, from the handshake through the life of the session.
fn handle_accepted(
    stream: TcpStream,
    hub: Hub,
    token: String,
    tls: Option<Arc<rustls::ServerConfig>>,
) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT));

    match tls {
        None => {
            let reader = match stream.try_clone() {
                Ok(clone) => clone,
                Err(_) => return,
            };
            // Clearing the deadline on any clone clears it for all of them: socket timeouts
            // are set on the socket, not the file descriptor.
            let clearer = match stream.try_clone() {
                Ok(clone) => clone,
                Err(_) => return,
            };
            // A clone kept only so the writer can `shutdown` the kernel socket. Dropping one
            // half does not send EOF while the reader still holds the other.
            let closer = match stream.try_clone() {
                Ok(clone) => clone,
                Err(_) => return,
            };
            handle_connection(
                Box::new(reader),
                Box::new(stream),
                Box::new(move || {
                    let _ = clearer.set_read_timeout(None);
                }),
                closer,
                hub,
                &token,
            );
        }
        Some(config) => {
            let mut conn = match rustls::ServerConnection::new(config) {
                Ok(conn) => conn,
                Err(_) => return,
            };
            let mut stream = stream;
            while conn.is_handshaking() {
                if conn.complete_io(&mut stream).is_err() {
                    return;
                }
            }
            let closer = match stream.try_clone() {
                Ok(clone) => clone,
                Err(_) => return,
            };
            let shared = SharedTls(Arc::new(Mutex::new(TlsPair { conn, sock: stream })));
            let clearer = shared.clone();
            handle_connection(
                Box::new(shared.clone()),
                Box::new(shared),
                Box::new(move || {
                    if let Ok(pair) = clearer.0.lock() {
                        let _ = pair.sock.set_read_timeout(None);
                    }
                }),
                closer,
                hub,
                &token,
            );
        }
    }
}

/// One connection, from the HTTP upgrade through the life of the session.
///
/// `on_upgrade` runs the moment the `101` is written: the handshake deadline was the
/// handshake's, not the session's, and a client that says nothing for an hour is an idle
/// window, not a stalled peer.
fn handle_connection(
    mut reader: Box<dyn Socket>,
    mut writer: Box<dyn Socket>,
    on_upgrade: Box<dyn FnOnce() + Send>,
    closer: TcpStream,
    hub: Hub,
    token: &str,
) {
    let request = match read_request(&mut reader) {
        Ok(request) => request,
        Err(_) => return,
    };

    match request {
        Request::Attach { token: presented } => {
            if !constant_time_eq(presented.as_bytes(), token.as_bytes()) {
                let _ = write_response(&mut writer, 401, "Unauthorized", "bad or missing token");
                return;
            }
            if write_response_line(&mut writer, "101 Switching Protocols").is_err() {
                return;
            }
            // The deadline was the handshake's, not the session's: a client that says nothing for
            // an hour is an idle window, not a stalled peer.
            on_upgrade();
            pump(reader, writer, closer, hub);
        }
        Request::Root => {
            let _ = write_response(
                &mut writer,
                200,
                "OK",
                "this is a Ubiq host. attach at /attach?token=<token>.",
            );
        }
        Request::Other => {
            let _ = write_response(&mut writer, 404, "Not Found", "not found");
        }
        Request::TooLarge => {
            let _ = write_response(&mut writer, 400, "Bad Request", "header too large");
        }
    }
}

enum Request {
    /// `GET /attach?token=<token>` with an `Upgrade: ubiq` header.
    Attach { token: String },
    /// A plain `GET /`.
    Root,
    /// Anything else recognisable as an HTTP request.
    Other,
    /// The header block ran past [`MAX_HEADER`] before a blank line ended it.
    TooLarge,
}

/// Read one HTTP request's header block with a hard cap, and pick out only what this handshake
/// needs: the request line's path, and whether an `Upgrade: ubiq` header was sent. The body, if
/// any, is never read — none of these requests has one worth reading.
///
/// Reads one byte at a time, directly off the socket, deliberately not through a `BufReader`: a
/// buffered reader can pull well past the header block's end in one underlying read, and anything
/// past the blank line is the first wire frame once this handshake upgrades — bytes this function
/// must leave on the socket for [`pump`] to read, not swallow into a buffer that is about to be
/// dropped. One byte per syscall is a one-time cost on a connection that then runs for a whole
/// session.
fn read_request(stream: &mut dyn Read) -> io::Result<Request> {
    let mut total = 0usize;
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        let n = read_capped_line(stream, &mut line, MAX_HEADER - total)?;
        match n {
            None => return Ok(Request::TooLarge),
            Some(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            Some(read) => total += read,
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        lines.push(trimmed.to_string());
        if total >= MAX_HEADER {
            return Ok(Request::TooLarge);
        }
    }

    let Some(request_line) = lines.first() else {
        return Ok(Request::Other);
    };
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(path)) = (parts.next(), parts.next()) else {
        return Ok(Request::Other);
    };
    if method != "GET" {
        return Ok(Request::Other);
    }

    if path == "/" {
        return Ok(Request::Root);
    }

    let Some(query) = path.strip_prefix("/attach?") else {
        return Ok(Request::Other);
    };
    let upgraded = lines.iter().skip(1).any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.trim().eq_ignore_ascii_case("upgrade") && value.trim().eq_ignore_ascii_case("ubiq")
        })
    });
    if !upgraded {
        return Ok(Request::Other);
    }
    let token = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("token="))
        .unwrap_or("");
    Ok(Request::Attach {
        token: token.to_string(),
    })
}

/// Read one line, refusing to grow the buffer past `budget` bytes. `Ok(None)` is "ran out of
/// budget before a newline"; `Ok(Some(0))` is a clean EOF with nothing read at all.
fn read_capped_line(
    reader: &mut dyn Read,
    line: &mut String,
    budget: usize,
) -> io::Result<Option<usize>> {
    let mut byte = [0u8; 1];
    let mut read = 0usize;
    loop {
        if read >= budget {
            return Ok(None);
        }
        match reader.read(&mut byte) {
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

fn write_response(stream: &mut dyn Write, code: u16, reason: &str, body: &str) -> io::Result<()> {
    let response = format!(
        "HTTP/1.1 {code} {reason}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    );
    stream.write_all(response.as_bytes())
}

fn write_response_line(stream: &mut dyn Write, status: &str) -> io::Result<()> {
    stream.write_all(
        format!("HTTP/1.1 {status}\r\nUpgrade: ubiq\r\nConnection: Upgrade\r\n\r\n").as_bytes(),
    )
}

/// How often the writer thread wakes to check whether the reader has given up, when nothing has
/// arrived from the coordinator to write. Bounds how long a dead read direction takes to end the
/// whole session; short enough nobody notices, long enough to cost nothing while both sides are
/// healthy.
const STOP_POLL: Duration = Duration::from_millis(200);

/// The session, once the handshake is done: attach a `Client`, and shuttle frames between it and
/// the socket until either side stops. This is the whole of what a remote client is — the same
/// `Client` a local window gets from `Hub::connect`, so every message family works with nothing
/// written here that knows what any of them mean.
///
/// The `Client` is shared behind an `Arc` because both directions need it — the reader calls
/// `send`, the writer drains `from_host` — and both take `&self`. It drops, telling the coordinator
/// `FromClient::Gone`, once both threads have released their half. A blocking `recv()` on the
/// writer's side would not notice the reader giving up (the coordinator has no reason to stop
/// answering just because the socket died), so the writer polls `from_host` instead and checks
/// `stopped`, which the reader sets on its way out.
fn pump(reader: Box<dyn Socket>, writer: Box<dyn Socket>, closer: TcpStream, hub: Hub) {
    let client = Arc::new(hub.connect());
    let stopped = Arc::new(AtomicBool::new(false));

    let mut reader_stream = reader;
    let mut writer_stream = writer;

    let writer_client = client.clone();
    let writer_stopped = stopped.clone();
    let writer = thread::Builder::new()
        .name("ubiq-remote-writer".to_string())
        .spawn(move || {
            loop {
                match writer_client.from_host().recv_timeout(STOP_POLL) {
                    Ok(message) => {
                        if wire::write_frame(&mut writer_stream, &message).is_err() {
                            break;
                        }
                    }
                    Err(flume::RecvTimeoutError::Timeout) => {
                        if writer_stopped.load(Ordering::Relaxed) {
                            break;
                        }
                    }
                    Err(flume::RecvTimeoutError::Disconnected) => break,
                }
            }
            // Shutdown the kernel socket while a handle still exists. Dropping a TLS
            // `SharedTls` or one TCP clone does not send EOF while the reader holds the
            // other half, so the host would never see `Gone` and would not reap panes.
            let _ = closer.shutdown(Shutdown::Both);
            drop(writer_stream);
            drop(writer_client);
        })
        .expect("the remote writer thread");

    while let Ok(message) = wire::read_frame(&mut reader_stream) {
        client.send(message);
    }
    stopped.store(true, Ordering::Relaxed);
    drop(client);

    let _ = writer.join();
}
