//! The HTTP kind: rewrite what names the upstream, both ways, so a browser on `localhost` can use
//! a far origin as if it were local.
//!
//! **Requests.** A streaming state machine over the bytes a browser sends. It parses a request
//! head with `httparse`, writes it back with `Host` replaced (or added) by the upstream's — and an
//! `Origin` / `Referer` naming the local origin by the upstream's — then copies the body by its
//! own framing — `Content-Length`, or `chunked` to its last-chunk and trailers — and parses the
//! next head after it, so pipelined and keep-alive requests are each rewritten. With
//! [`TunnelSpec::rewrite_host`] off, those three pass as the browser sent them. After a `CONNECT`
//! the rest of the stream passes untouched. After a head that asks for an upgrade (a websocket)
//! the server may still refuse and stay HTTP: so the next bytes are still read as a request head,
//! and only bytes that cannot start one — a websocket frame, an HTTP/2 preface — turn the stream
//! raw. A client sends nothing after an upgrade request until the answer, so the first bytes after
//! it are either the new protocol or the next request.
//!
//! **Responses.** The same machine the other way, paired with the request side through a queue of
//! the requests still unanswered (a `HEAD` has no body, a `CONNECT`'s `2xx` is a raw stream, and
//! each request's own `Host` is the local origin its answer is rewritten to). The headers that
//! name the upstream — `Location`, `Content-Location`, `Refresh`, `Link`,
//! `Access-Control-Allow-Origin` — have its origin replaced by the local one, so a redirect comes
//! back through the tunnel; a `Set-Cookie` loses its `Domain`, so the cookie lands on `localhost`.
//! A `101` or a `CONNECT`'s `2xx` turns the stream raw; so does anything that does not parse.
//!
//! **Bodies** ([`TunnelSpec::rewrite_content`], experimental). A response whose `Content-Type`
//! matches one of [`TunnelSpec::content_types`] and carries no `Content-Encoding` is held whole —
//! decoded from `chunked` if it came so — has the upstream's origin replaced by the local one (and
//! the JSON-escaped `https:\/\/` spelling too), and goes out with a fresh `Content-Length`. The
//! request side asks for `Accept-Encoding: identity` so a server has no reason to compress. A
//! body that outgrows [`CAPTURE_MAX`] goes out as it came, unrewritten, and `text/event-stream`
//! is never held.
//!
//! **Refusals.** A request the guard refuses (`403`) or whose framing is ambiguous — a
//! `Transfer-Encoding` other than `chunked`, two `Content-Length`s that disagree (`400`) — forwards
//! nothing more. Its answer is not written by the request side: it waits for the response side to
//! finish what the upstream owes for the requests before it, so it never lands inside one.
//!
//! **DNS rebinding.** Every request's incoming `Host` must name the loopback — `localhost`,
//! `127.0.0.1` or `[::1]`, any port — or the stream is refused with a `403`: a page on a hostile
//! name that resolves to `127.0.0.1` would otherwise reach the far service with the browser's
//! blessing. A request with no `Host` (HTTP/1.0) passes.

use std::collections::VecDeque;
use std::io;
use std::sync::Arc;

use httparse::Status;
use parking_lot::Mutex;
use ubiq_proto::settings::TunnelSpec;

/// The longest request or response head accepted.
const HEAD_MAX: usize = 64 * 1024;
/// The longest chunk-size or trailer line accepted.
const LINE_MAX: usize = 8 * 1024;
const MAX_HEADERS: usize = 128;
/// The largest response body held for a content rewrite; a larger one passes unrewritten.
const CAPTURE_MAX: usize = 2 * 1024 * 1024;

/// What one HTTP spec rewrites, both ways.
pub(crate) struct Rewrite {
    /// The `Host` sent upstream; `None` passes the browser's.
    host: Option<String>,
    /// The upstream's origin, `https://api.x`.
    remote_origin: String,
    /// Every way a response may spell the upstream's origin, the explicit port first.
    remote_spellings: Vec<String>,
    /// With a content rewrite: the lowercased `Content-Type` patterns it processes.
    content_types: Option<Vec<String>>,
    /// The local origin when a request named no `Host`.
    fallback_origin: String,
}

impl Rewrite {
    pub(crate) fn new(spec: &TunnelSpec) -> Self {
        let remote_origin = spec.remote_origin();
        let scheme = if spec.upstream_tls { "https" } else { "http" };
        let host = if spec.host.contains(':') && !spec.host.starts_with('[') {
            format!("[{}]", spec.host)
        } else {
            spec.host.clone()
        };
        let explicit = format!("{scheme}://{host}:{}", spec.port);
        let mut remote_spellings = vec![explicit.clone()];
        if remote_origin != explicit {
            remote_spellings.push(remote_origin.clone());
        }
        Self {
            host: spec.rewrite_host.then(|| {
                spec.host_rewrite
                    .as_deref()
                    .map(str::trim)
                    .filter(|h| !h.is_empty())
                    .map_or_else(|| spec.remote_authority(), String::from)
            }),
            remote_origin,
            remote_spellings,
            content_types: spec.rewrite_content.then(|| {
                spec.content_types
                    .iter()
                    .map(|p| p.trim().to_ascii_lowercase())
                    .filter(|p| !p.is_empty())
                    .collect()
            }),
            fallback_origin: format!("http://localhost:{}", spec.local_port),
        }
    }

    fn rewrites_content(&self, content_type: &str) -> bool {
        let essence = content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        // A stream of events never ends, so holding it whole would hold it forever.
        if essence == "text/event-stream" {
            return false;
        }
        self.content_types.as_ref().is_some_and(|patterns| {
            patterns
                .iter()
                .any(|p| glob(p.as_bytes(), essence.as_bytes()))
        })
    }

    /// The upstream's spellings paired with `local`; with `escaped`, the JSON `\/` ones too.
    fn pairs(&self, local: &str, escaped: bool) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = self
            .remote_spellings
            .iter()
            .map(|s| (s.clone().into_bytes(), local.as_bytes().to_vec()))
            .collect();
        if escaped {
            let esc = |s: &str| s.replace('/', "\\/").into_bytes();
            pairs.extend(self.remote_spellings.iter().map(|s| (esc(s), esc(local))));
        }
        pairs
    }
}

/// A request sent and not yet answered: what its response needs to know.
#[derive(Clone)]
struct Exchange {
    /// A `HEAD`: the response has no body, whatever its headers say.
    head: bool,
    connect: bool,
    /// `http://` and the request's own `Host`: what the upstream's origin becomes.
    local_origin: String,
}

/// What a connection's two directions share.
#[derive(Default)]
struct Link {
    /// Requests sent and not yet answered, oldest first.
    exchanges: Mutex<VecDeque<Exchange>>,
    /// The answer to a refused request, for the response side to send once it is idle.
    refusal: Mutex<Option<Vec<u8>>>,
}

type Exchanges = Arc<Link>;

/// A connection's two rewriters, sharing its queue of unanswered requests.
pub(crate) fn rewriters(rewrite: Rewrite) -> (RequestRewriter, ResponseRewriter) {
    let rewrite = Arc::new(rewrite);
    let exchanges = Exchanges::default();
    (
        RequestRewriter {
            rewrite: rewrite.clone(),
            exchanges: exchanges.clone(),
            state: State::Head,
            buf: Vec::new(),
        },
        ResponseRewriter {
            rewrite,
            exchanges,
            state: State::Head,
            buf: Vec::new(),
            capture: None,
        },
    )
}

enum State {
    Head,
    Body(u64),
    ChunkSize,
    /// The chunk's bytes plus its trailing CRLF.
    ChunkData(u64),
    ChunkTrailer,
    /// A response with no length: its body runs to the end of the stream.
    UntilEof,
    /// After an upgrade request: a head if the bytes parse as one, [`State::Raw`] if they cannot.
    Upgrading,
    /// An upgraded or `CONNECT` stream: no more HTTP.
    Raw,
}

fn after_body(left: u64) -> State {
    if left == 0 {
        State::Head
    } else {
        State::Body(left)
    }
}

/// The browser-to-upstream direction.
pub(crate) struct RequestRewriter {
    rewrite: Arc<Rewrite>,
    exchanges: Exchanges,
    state: State,
    buf: Vec<u8>,
}

impl RequestRewriter {
    /// Feed bytes read from the client; append what to send upstream to `out`. After an error
    /// nothing more may be fed; a refusal's answer is left for [`ResponseRewriter::farewell`].
    pub(crate) fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
        let result = self.feed_inner(input, out);
        if let Err(e) = &result
            && let Some(answer) = refusal_answer(e)
        {
            *self.exchanges.refusal.lock() = Some(answer);
        }
        result
    }

    fn feed_inner(&mut self, input: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
        if self.buf.is_empty() {
            match self.state {
                State::Raw => {
                    out.extend_from_slice(input);
                    return Ok(());
                }
                State::Body(n) if n as usize >= input.len() => {
                    out.extend_from_slice(input);
                    self.state = after_body(n - input.len() as u64);
                    return Ok(());
                }
                _ => {}
            }
        }
        self.buf.extend_from_slice(input);
        let mut pos = 0;
        while pos < self.buf.len() {
            let rest = &self.buf[pos..];
            match self.state {
                State::Raw | State::UntilEof => {
                    out.extend_from_slice(rest);
                    pos = self.buf.len();
                }
                State::Body(n) => {
                    let k = (n as usize).min(rest.len());
                    out.extend_from_slice(&rest[..k]);
                    pos += k;
                    self.state = after_body(n - k as u64);
                }
                State::ChunkData(n) => {
                    let k = (n as usize).min(rest.len());
                    out.extend_from_slice(&rest[..k]);
                    pos += k;
                    self.state = if n - k as u64 == 0 {
                        State::ChunkSize
                    } else {
                        State::ChunkData(n - k as u64)
                    };
                }
                State::ChunkSize => {
                    let Some(end) = line_end(rest, LINE_MAX)? else {
                        break;
                    };
                    let size = chunk_size(&rest[..end])?;
                    out.extend_from_slice(&rest[..end + 2]);
                    pos += end + 2;
                    self.state = next_chunk(size)?;
                }
                State::ChunkTrailer => {
                    let Some(end) = line_end(rest, LINE_MAX)? else {
                        break;
                    };
                    out.extend_from_slice(&rest[..end + 2]);
                    pos += end + 2;
                    if end == 0 {
                        self.state = State::Head;
                    }
                }
                State::Head | State::Upgrading => {
                    // A stray CRLF between requests is allowed; pass it on.
                    if matches!(self.state, State::Head) && rest.starts_with(b"\r\n") {
                        out.extend_from_slice(b"\r\n");
                        pos += 2;
                        continue;
                    }
                    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
                    let mut req = httparse::Request::new(&mut headers);
                    match req.parse(rest) {
                        Ok(Status::Partial) => {
                            if rest.len() > HEAD_MAX {
                                return Err(invalid("the request head is too large"));
                            }
                            break;
                        }
                        Ok(Status::Complete(n)) => {
                            self.state =
                                request_head(&self.rewrite, &self.exchanges, rest, &req, out)?;
                            pos += n;
                        }
                        // The server switched protocols: the rest is the new one's.
                        Err(_) if matches!(self.state, State::Upgrading) => {
                            self.state = State::Raw;
                        }
                        Err(e) => return Err(invalid(&format!("not an HTTP request: {e}"))),
                    }
                }
            }
        }
        self.buf.drain(..pos);
        Ok(())
    }
}

fn next_chunk(size: u64) -> io::Result<State> {
    let with_crlf = size
        .checked_add(2)
        .ok_or_else(|| invalid("a chunk size too large"))?;
    Ok(if size == 0 {
        State::ChunkTrailer
    } else {
        State::ChunkData(with_crlf)
    })
}

/// Write a request head rewritten, queue what its response needs, and say what follows it.
fn request_head(
    rewrite: &Rewrite,
    exchanges: &Exchanges,
    raw: &[u8],
    req: &httparse::Request,
    out: &mut Vec<u8>,
) -> io::Result<State> {
    let mut client_host = None;
    for h in req.headers.iter() {
        if h.name.eq_ignore_ascii_case("host") {
            if !is_loopback_host(h.value) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "a request for {} was refused: only localhost may use a tunnel",
                        String::from_utf8_lossy(h.value).trim()
                    ),
                ));
            }
            client_host.get_or_insert_with(|| String::from_utf8_lossy(h.value).trim().to_string());
        }
    }
    let local_origin = client_host.as_ref().map_or_else(
        || rewrite.fallback_origin.clone(),
        |h| format!("http://{h}"),
    );

    // The request line verbatim — the target is the client's, however it spelled it.
    let line = raw
        .iter()
        .position(|&b| b == b'\n')
        .map_or(raw.len(), |i| i + 1);
    out.extend_from_slice(&raw[..line]);
    let mut wrote_host = false;
    let mut length: Option<u64> = None;
    let mut chunked = false;
    let mut upgrade = false;
    let mut connection_upgrade = false;
    for h in req.headers.iter() {
        let name = h.name;
        if name.eq_ignore_ascii_case("host") {
            match &rewrite.host {
                Some(host) if !wrote_host => push_header(out, "Host", host.as_bytes()),
                Some(_) => {}
                None => push_header(out, name, h.value),
            }
            wrote_host = true;
            continue;
        }
        if rewrite.content_types.is_some() && name.eq_ignore_ascii_case("accept-encoding") {
            continue;
        }
        if rewrite.host.is_some()
            && (name.eq_ignore_ascii_case("origin") || name.eq_ignore_ascii_case("referer"))
            && let Some(v) = swap_prefix(h.value, &local_origin, &rewrite.remote_origin)
        {
            push_header(out, name, &v);
            continue;
        }
        if name.eq_ignore_ascii_case("content-length") {
            let text = std::str::from_utf8(h.value).unwrap_or("").trim();
            let n: u64 = text
                .parse()
                .map_err(|_| bad_request("a Content-Length that is not a number"))?;
            if length.is_some_and(|l| l != n) {
                return Err(bad_request("two Content-Lengths that disagree"));
            }
            length = Some(n);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            let text = String::from_utf8_lossy(h.value).to_ascii_lowercase();
            if text.rsplit(',').next().map(str::trim) != Some("chunked") {
                return Err(bad_request(
                    "a Transfer-Encoding that does not end in chunked",
                ));
            }
            chunked = true;
        } else if name.eq_ignore_ascii_case("upgrade") {
            upgrade = true;
        } else if name.eq_ignore_ascii_case("connection") {
            connection_upgrade |= String::from_utf8_lossy(h.value)
                .to_ascii_lowercase()
                .split(',')
                .any(|t| t.trim() == "upgrade");
        }
        push_header(out, name, h.value);
    }
    if !wrote_host && let Some(host) = &rewrite.host {
        push_header(out, "Host", host.as_bytes());
    }
    if rewrite.content_types.is_some() {
        // A body to rewrite has to arrive readable.
        push_header(out, "Accept-Encoding", b"identity");
    }
    out.extend_from_slice(b"\r\n");
    let connect = req.method == Some("CONNECT");
    exchanges.exchanges.lock().push_back(Exchange {
        head: req.method == Some("HEAD"),
        connect,
        local_origin,
    });
    Ok(if connect {
        State::Raw
    } else if upgrade && connection_upgrade {
        State::Upgrading
    } else if chunked {
        State::ChunkSize
    } else {
        after_body(length.unwrap_or(0))
    })
}

/// A response body held for a content rewrite.
struct Capture {
    /// The rewritten head with its own framing headers and closing blank line: what goes out,
    /// followed by `raw`, if the body is never rewritten.
    passthrough: Vec<u8>,
    /// The rewritten head without its framing headers or its closing blank line.
    head: Vec<u8>,
    /// The body's bytes as they came, framing included.
    raw: Vec<u8>,
    /// The body decoded.
    body: Vec<u8>,
    local_origin: String,
}

/// The upstream-to-browser direction. Never fails: what it cannot parse it passes on.
pub(crate) struct ResponseRewriter {
    rewrite: Arc<Rewrite>,
    exchanges: Exchanges,
    state: State,
    buf: Vec<u8>,
    capture: Option<Capture>,
}

impl ResponseRewriter {
    /// Feed bytes from the upstream; append what to send to the browser to `out`.
    pub(crate) fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) {
        if self.buf.is_empty() && self.capture.is_none() {
            match self.state {
                State::Raw | State::UntilEof => {
                    out.extend_from_slice(input);
                    return;
                }
                State::Body(n) if n as usize >= input.len() => {
                    out.extend_from_slice(input);
                    self.state = after_body(n - input.len() as u64);
                    return;
                }
                _ => {}
            }
        }
        self.buf.extend_from_slice(input);
        let mut pos = 0;
        while pos < self.buf.len() {
            let rest = &self.buf[pos..];
            match self.state {
                State::Raw | State::Upgrading => {
                    out.extend_from_slice(rest);
                    pos = self.buf.len();
                }
                State::UntilEof => {
                    pass(&mut self.capture, out, rest, rest);
                    pos = self.buf.len();
                }
                State::Body(n) => {
                    let k = (n as usize).min(rest.len());
                    pass(&mut self.capture, out, &rest[..k], &rest[..k]);
                    pos += k;
                    self.state = after_body(n - k as u64);
                    if matches!(self.state, State::Head) {
                        finish_body(&mut self.capture, &self.rewrite, out);
                    }
                }
                State::ChunkData(n) => {
                    let k = (n as usize).min(rest.len());
                    let payload = k.min(n.saturating_sub(2) as usize);
                    pass(&mut self.capture, out, &rest[..k], &rest[..payload]);
                    pos += k;
                    self.state = if n - k as u64 == 0 {
                        State::ChunkSize
                    } else {
                        State::ChunkData(n - k as u64)
                    };
                }
                State::ChunkSize | State::ChunkTrailer => {
                    // A framing line this cannot read: stop parsing, pass the rest as it comes.
                    let end = match line_end(rest, LINE_MAX) {
                        Ok(Some(end)) => end,
                        Ok(None) => break,
                        Err(_) => {
                            release(&mut self.capture, out);
                            self.state = State::Raw;
                            continue;
                        }
                    };
                    let state = if matches!(self.state, State::ChunkTrailer) {
                        if end == 0 {
                            State::Head
                        } else {
                            State::ChunkTrailer
                        }
                    } else {
                        match chunk_size(&rest[..end]).and_then(next_chunk) {
                            Ok(state) => state,
                            Err(_) => {
                                release(&mut self.capture, out);
                                self.state = State::Raw;
                                continue;
                            }
                        }
                    };
                    pass(&mut self.capture, out, &rest[..end + 2], &[]);
                    pos += end + 2;
                    self.state = state;
                    if matches!(self.state, State::Head) {
                        finish_body(&mut self.capture, &self.rewrite, out);
                    }
                }
                State::Head => {
                    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
                    let mut res = httparse::Response::new(&mut headers);
                    match res.parse(rest) {
                        Ok(Status::Partial) if rest.len() <= HEAD_MAX => break,
                        Ok(Status::Complete(n)) => {
                            self.state = response_head(
                                &self.rewrite,
                                &self.exchanges,
                                &mut self.capture,
                                rest,
                                &res,
                                out,
                            );
                            pos += n;
                        }
                        // Not HTTP as this understands it: pass the rest as it comes.
                        _ => self.state = State::Raw,
                    }
                }
            }
        }
        self.buf.drain(..pos);
    }

    /// The upstream closed: a body that ran to the end of the stream is complete now.
    pub(crate) fn finish(&mut self, out: &mut Vec<u8>) {
        if matches!(self.state, State::UntilEof) {
            finish_body(&mut self.capture, &self.rewrite, out);
        }
        // A truncated response goes out as far as it came.
        release(&mut self.capture, out);
        out.append(&mut self.buf);
        self.state = State::Raw;
        if let Some(answer) = self.exchanges.refusal.lock().take() {
            out.extend_from_slice(&answer);
        }
    }

    /// A refused request's answer, once every request before it has had its response in full.
    /// The caller writes it and closes the connection.
    pub(crate) fn farewell(&mut self) -> Option<Vec<u8>> {
        let idle = matches!(self.state, State::Head)
            && self.buf.is_empty()
            && self.capture.is_none()
            && self.exchanges.exchanges.lock().is_empty();
        if idle {
            self.exchanges.refusal.lock().take()
        } else {
            None
        }
    }
}

/// Send a held body as it came, unrewritten.
fn release(capture: &mut Option<Capture>, out: &mut Vec<u8>) {
    if let Some(c) = capture.take() {
        out.extend_from_slice(&c.passthrough);
        out.extend_from_slice(&c.raw);
    }
}

/// Body bytes: held when a capture is open, sent otherwise. A capture that outgrows its cap is
/// sent as it came and dropped.
fn pass(capture: &mut Option<Capture>, out: &mut Vec<u8>, raw: &[u8], payload: &[u8]) {
    let Some(c) = capture else {
        out.extend_from_slice(raw);
        return;
    };
    c.raw.extend_from_slice(raw);
    c.body.extend_from_slice(payload);
    if c.raw.len() > CAPTURE_MAX {
        release(capture, out);
    }
}

/// A held body is complete: send its head with a fresh length, then the rewritten body.
fn finish_body(capture: &mut Option<Capture>, rewrite: &Rewrite, out: &mut Vec<u8>) {
    let Some(c) = capture.take() else {
        return;
    };
    let pairs = rewrite.pairs(&c.local_origin, true);
    let body = replace_origins(&c.body, &pairs).unwrap_or(c.body);
    out.extend_from_slice(&c.head);
    push_header(out, "Content-Length", body.len().to_string().as_bytes());
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&body);
}

/// Write a response head rewritten — or open a capture for it — and say what follows it.
fn response_head(
    rewrite: &Rewrite,
    exchanges: &Exchanges,
    capture: &mut Option<Capture>,
    raw: &[u8],
    res: &httparse::Response,
    out: &mut Vec<u8>,
) -> State {
    let code = res.code.unwrap_or(0);
    let interim = (100..200).contains(&code) && code != 101;
    let exchange = {
        let mut queue = exchanges.exchanges.lock();
        let next = if interim {
            queue.front().cloned()
        } else {
            queue.pop_front()
        };
        next.unwrap_or_else(|| Exchange {
            head: false,
            connect: false,
            local_origin: rewrite.fallback_origin.clone(),
        })
    };
    let pairs = rewrite.pairs(&exchange.local_origin, false);

    let line = raw
        .iter()
        .position(|&b| b == b'\n')
        .map_or(raw.len(), |i| i + 1);
    let mut head = raw[..line].to_vec();
    let mut framing = Vec::new();
    let mut length: Option<u64> = None;
    let mut chunked = false;
    let mut te = false;
    let mut content_type = String::new();
    let mut encoded = false;
    for h in res.headers.iter() {
        let name = h.name;
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "content-length" => {
                length = std::str::from_utf8(h.value)
                    .ok()
                    .and_then(|t| t.trim().parse().ok());
                push_header(&mut framing, name, h.value);
            }
            "transfer-encoding" => {
                te = true;
                let text = String::from_utf8_lossy(h.value).to_ascii_lowercase();
                chunked = text.rsplit(',').next().map(str::trim) == Some("chunked");
                push_header(&mut framing, name, h.value);
            }
            "content-type" => {
                content_type = String::from_utf8_lossy(h.value).into_owned();
                push_header(&mut head, name, h.value);
            }
            "content-encoding" => {
                encoded |= !String::from_utf8_lossy(h.value)
                    .trim()
                    .eq_ignore_ascii_case("identity");
                push_header(&mut head, name, h.value);
            }
            "location"
            | "content-location"
            | "refresh"
            | "link"
            | "access-control-allow-origin" => {
                let value = replace_origins(h.value, &pairs);
                push_header(&mut head, name, value.as_deref().unwrap_or(h.value));
            }
            "set-cookie" => {
                let value = strip_cookie_domain(h.value);
                push_header(&mut head, name, value.as_deref().unwrap_or(h.value));
            }
            _ => push_header(&mut head, name, h.value),
        }
    }

    let state = if code == 101 || (exchange.connect && (200..300).contains(&code)) {
        State::Raw
    } else if (100..200).contains(&code) || exchange.head || code == 204 || code == 304 {
        State::Head
    } else if chunked {
        State::ChunkSize
    } else if let (Some(n), false) = (length, te) {
        after_body(n)
    } else {
        State::UntilEof
    };
    let has_body = matches!(state, State::Body(_) | State::ChunkSize | State::UntilEof);
    let mut passthrough = head.clone();
    passthrough.extend_from_slice(&framing);
    passthrough.extend_from_slice(b"\r\n");
    if has_body && !encoded && rewrite.rewrites_content(&content_type) {
        *capture = Some(Capture {
            passthrough,
            head,
            raw: Vec::new(),
            body: Vec::new(),
            local_origin: exchange.local_origin,
        });
    } else {
        out.extend_from_slice(&passthrough);
    }
    state
}

/// Replace every spelling of an origin in `hay` with its partner — only whole, so neither a
/// longer host (`api.x.evil`) nor another port matches. `None` when nothing did.
fn replace_origins(hay: &[u8], pairs: &[(Vec<u8>, Vec<u8>)]) -> Option<Vec<u8>> {
    let mut out: Option<Vec<u8>> = None;
    let mut copied = 0;
    let mut i = 0;
    while i < hay.len() {
        let found = pairs.iter().find(|(from, _)| {
            hay.len() - i >= from.len()
                && hay[i..i + from.len()].eq_ignore_ascii_case(from)
                && hay.get(i + from.len()).is_none_or(|&b| {
                    !(b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':' | b'@'))
                })
        });
        match found {
            Some((from, to)) => {
                let o = out.get_or_insert_with(|| Vec::with_capacity(hay.len()));
                o.extend_from_slice(&hay[copied..i]);
                o.extend_from_slice(to);
                i += from.len();
                copied = i;
            }
            None => i += 1,
        }
    }
    let mut out = out?;
    out.extend_from_slice(&hay[copied..]);
    Some(out)
}

/// A request header naming `from` as its origin — `Origin` exactly, `Referer` as a prefix —
/// rewritten to `to`.
fn swap_prefix(value: &[u8], from: &str, to: &str) -> Option<Vec<u8>> {
    let from = from.as_bytes();
    let head = value.get(..from.len())?;
    if !head.eq_ignore_ascii_case(from) {
        return None;
    }
    let rest = &value[from.len()..];
    if !(rest.is_empty() || matches!(rest[0], b'/' | b'?' | b'#')) {
        return None;
    }
    let mut out = to.as_bytes().to_vec();
    out.extend_from_slice(rest);
    Some(out)
}

/// A `Set-Cookie` without its `Domain` attribute, so the browser keeps it for `localhost`.
fn strip_cookie_domain(value: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(value).ok()?;
    let mut parts = text.split(';');
    let first = parts.next()?;
    let mut dropped = false;
    let mut out = first.to_string();
    for part in parts {
        if part
            .trim_start()
            .get(..7)
            .is_some_and(|p| p.eq_ignore_ascii_case("domain="))
        {
            dropped = true;
            continue;
        }
        out.push(';');
        out.push_str(part);
    }
    dropped.then(|| out.into_bytes())
}

/// `*` matches any run of characters; everything else matches itself.
fn glob(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && pattern[p] == b'*' {
            star = Some((p, t));
            p += 1;
        } else if p < pattern.len() && pattern[p] == text[t] {
            p += 1;
            t += 1;
        } else if let Some((sp, st)) = star {
            p = sp + 1;
            t = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&b| b == b'*')
}

/// Whether an incoming `Host` names the loopback: `localhost`, `127.0.0.1` or `[::1]`, with or
/// without a port.
fn is_loopback_host(value: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(value) else {
        return false;
    };
    let text = text.trim().to_ascii_lowercase();
    let (host, port) = match text.strip_prefix('[') {
        Some(v6) => match v6.split_once(']') {
            Some((addr, port)) => (format!("[{addr}]"), port.to_string()),
            None => return false,
        },
        None => match text.split_once(':') {
            Some((host, port)) => (host.to_string(), format!(":{port}")),
            None => (text.clone(), String::new()),
        },
    };
    let port_ok = port.is_empty()
        || port
            .strip_prefix(':')
            .is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    port_ok
        && matches!(
            host.trim_end_matches('.'),
            "localhost" | "127.0.0.1" | "[::1]"
        )
}

/// Whether a request-side error is a refusal the browser is answered for — `403` for the
/// rebinding guard, `400` for ambiguous framing — rather than a stream that just ends.
pub(crate) fn is_refusal(error: &io::Error) -> bool {
    refusal_status(error).is_some()
}

fn refusal_status(error: &io::Error) -> Option<&'static str> {
    match error.kind() {
        io::ErrorKind::PermissionDenied => Some("403 Forbidden"),
        io::ErrorKind::InvalidInput => Some("400 Bad Request"),
        _ => None,
    }
}

fn refusal_answer(error: &io::Error) -> Option<Vec<u8>> {
    let status = refusal_status(error)?;
    let body = format!("ubiq tunnel: {error}\n");
    Some(
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes(),
    )
}

fn bad_request(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.to_string())
}

fn push_header(out: &mut Vec<u8>, name: &str, value: &[u8]) {
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(b": ");
    out.extend_from_slice(value);
    out.extend_from_slice(b"\r\n");
}

/// Where the CRLF-terminated line at the start of `bytes` ends, without the CRLF.
fn line_end(bytes: &[u8], max: usize) -> io::Result<Option<usize>> {
    match bytes.windows(2).position(|w| w == b"\r\n") {
        Some(i) => Ok(Some(i)),
        None if bytes.len() > max => Err(invalid("a chunked line is too long")),
        None => Ok(None),
    }
}

fn chunk_size(line: &[u8]) -> io::Result<u64> {
    let text = std::str::from_utf8(line).map_err(|_| invalid("a chunk size that is not text"))?;
    let hex = text.split(';').next().unwrap_or("").trim();
    u64::from_str_radix(hex, 16).map_err(|_| invalid("a chunk size that is not hexadecimal"))
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::settings::TunnelKind;

    fn spec(url: &str) -> TunnelSpec {
        let mut spec = TunnelSpec {
            id: "t".into(),
            kind: TunnelKind::Http,
            local_port: 8080,
            host: String::new(),
            port: 0,
            upstream_tls: false,
            insecure: false,
            enabled: true,
            rewrite_host: true,
            host_rewrite: None,
            rewrite_content: false,
            content_types: TunnelSpec::default_content_types(),
        };
        spec.set_remote_url(url).unwrap();
        spec
    }

    fn requests(spec: &TunnelSpec) -> RequestRewriter {
        rewriters(Rewrite::new(spec)).0
    }

    fn run(input: &[u8], step: usize) -> Vec<u8> {
        let mut r = requests(&spec("http://api.example.com"));
        let mut out = Vec::new();
        for piece in input.chunks(step) {
            r.feed(piece, &mut out).unwrap();
        }
        out
    }

    /// One request through, then the response fed in `step`-byte pieces, then the stream's end.
    fn answer(spec: &TunnelSpec, request: &[u8], response: &[u8], step: usize) -> (String, String) {
        let (mut up, mut down) = rewriters(Rewrite::new(spec));
        let mut sent = Vec::new();
        up.feed(request, &mut sent).unwrap();
        let mut out = Vec::new();
        for piece in response.chunks(step) {
            down.feed(piece, &mut out);
        }
        down.finish(&mut out);
        (
            String::from_utf8_lossy(&sent).into_owned(),
            String::from_utf8_lossy(&out).into_owned(),
        )
    }

    #[test]
    fn pipelined_requests_are_each_rewritten_and_bodies_are_not() {
        let input = b"POST /a HTTP/1.1\r\nHost: localhost:8080\r\nContent-Length: 18\r\n\r\nHost: not-a-header\
GET /b HTTP/1.1\r\nhost: localhost:8080\r\nAccept: */*\r\n\r\n";
        let want = b"POST /a HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: 18\r\n\r\nHost: not-a-header\
GET /b HTTP/1.1\r\nHost: api.example.com\r\nAccept: */*\r\n\r\n";
        for step in [1, 3, 7, input.len()] {
            assert_eq!(
                String::from_utf8_lossy(&run(input, step)),
                String::from_utf8_lossy(want)
            );
        }
    }

    #[test]
    fn a_chunked_body_is_copied_to_its_trailer_then_the_next_head_parses() {
        let input = b"POST /up HTTP/1.1\r\nHost: 127.0.0.1\r\nTransfer-Encoding: chunked\r\n\r\n\
5;ext=1\r\nHost:\r\n0\r\nX-Trailer: 1\r\n\r\nGET / HTTP/1.1\r\n\r\n";
        let want =
            b"POST /up HTTP/1.1\r\nHost: api.example.com\r\nTransfer-Encoding: chunked\r\n\r\n\
5;ext=1\r\nHost:\r\n0\r\nX-Trailer: 1\r\n\r\nGET / HTTP/1.1\r\nHost: api.example.com\r\n\r\n";
        for step in [1, 2, 5, input.len()] {
            assert_eq!(
                String::from_utf8_lossy(&run(input, step)),
                String::from_utf8_lossy(want)
            );
        }
    }

    #[test]
    fn an_upgrade_passes_the_rest_through() {
        let input =
            b"GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n\
\x81\x05hello GET /not-a-request HTTP/1.1\r\nHost: x\r\n\r\n";
        let head = b"GET /ws HTTP/1.1\r\nHost: api.example.com\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n";
        let first = input.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        for step in [1, 4, input.len()] {
            let out = run(input, step);
            assert!(out.starts_with(head));
            assert_eq!(&out[head.len()..], &input[first..]);
        }
    }

    #[test]
    fn a_refused_upgrade_keeps_rewriting_the_next_request() {
        let input =
            b"GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n\
GET /b HTTP/1.1\r\nHost: localhost:3000\r\n\r\n";
        let want = b"GET /ws HTTP/1.1\r\nHost: api.example.com\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n\
GET /b HTTP/1.1\r\nHost: api.example.com\r\n\r\n";
        for step in [1, 4, input.len()] {
            assert_eq!(
                String::from_utf8_lossy(&run(input, step)),
                String::from_utf8_lossy(want)
            );
        }
        // And still guards it.
        let mut r = requests(&spec("http://h"));
        let mut out = Vec::new();
        r.feed(
            b"GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n",
            &mut out,
        )
        .unwrap();
        let e = r
            .feed(b"GET / HTTP/1.1\r\nHost: evil.example\r\n\r\n", &mut out)
            .unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn the_host_header_leaves_the_scheme_port_off_and_brackets_v6() {
        let host = |url: &str| Rewrite::new(&spec(url)).host.unwrap();
        assert_eq!(host("https://api.x"), "api.x");
        assert_eq!(host("http://api.x:443"), "api.x:443");
        assert_eq!(host("http://[::1]:8080"), "[::1]:8080");
        let mut s = spec("http://10.0.0.1");
        s.host_rewrite = Some(" svc.local ".into());
        assert_eq!(Rewrite::new(&s).host.as_deref(), Some("svc.local"));
        s.host_rewrite = Some(" ".into());
        assert_eq!(Rewrite::new(&s).host.as_deref(), Some("10.0.0.1"));
        s.host_rewrite = Some("svc.local".into());
        s.rewrite_host = false;
        assert_eq!(Rewrite::new(&s).host, None);
    }

    #[test]
    fn ambiguous_framing_is_a_400_and_forwards_nothing() {
        for head in [
            &b"POST / HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: gzip\r\n\r\n"[..],
            b"POST / HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked, gzip\r\n\r\n",
            b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 3\r\nContent-Length: 4\r\n\r\n",
            b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: x\r\n\r\n",
        ] {
            let (mut up, mut down) = rewriters(Rewrite::new(&spec("http://web")));
            let mut out = Vec::new();
            // On an error the caller forwards nothing of `out`.
            let e = up.feed(head, &mut out).unwrap_err();
            assert!(is_refusal(&e));
            let answer = down.farewell().unwrap();
            assert!(answer.starts_with(b"HTTP/1.1 400 Bad Request\r\n"));
        }
        // The same length twice is not a conflict.
        let mut up = requests(&spec("http://web"));
        up.feed(
            b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 3\r\nContent-Length: 3\r\n\r\nabc",
            &mut Vec::new(),
        )
        .unwrap();
    }

    #[test]
    fn a_refusal_waits_for_the_responses_owed_before_it() {
        let (mut up, mut down) = rewriters(Rewrite::new(&spec("http://web")));
        let mut sent = Vec::new();
        up.feed(b"GET /a HTTP/1.1\r\nHost: localhost\r\n\r\n", &mut sent)
            .unwrap();
        let e = up
            .feed(b"GET /b HTTP/1.1\r\nHost: evil.example\r\n\r\n", &mut sent)
            .unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(down.farewell(), None, "the response to /a is still owed");
        let mut out = Vec::new();
        down.feed(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nab", &mut out);
        assert_eq!(down.farewell(), None, "half a body is not an answer");
        down.feed(b"cd", &mut out);
        let answer = down.farewell().unwrap();
        assert!(out.ends_with(b"abcd"));
        assert!(answer.starts_with(b"HTTP/1.1 403 Forbidden\r\n"));
        // At the upstream's end it goes out too, once.
        let (mut up, mut down) = rewriters(Rewrite::new(&spec("http://web")));
        let _ = up.feed(b"GET / HTTP/1.1\r\nHost: evil\r\n\r\n", &mut Vec::new());
        let mut out = Vec::new();
        down.finish(&mut out);
        assert!(out.starts_with(b"HTTP/1.1 403"));
        assert_eq!(down.farewell(), None);
    }

    #[test]
    fn origin_and_referer_name_the_upstream_and_the_flag_turns_all_three_off() {
        let req = b"POST / HTTP/1.1\r\nHost: localhost:8080\r\nOrigin: http://localhost:8080\r\n\
Referer: http://localhost:8080/login?x=1\r\nAccept-Encoding: gzip\r\n\r\n";
        let (sent, _) = answer(&spec("https://api.x"), req, b"", 1);
        assert_eq!(
            sent,
            "POST / HTTP/1.1\r\nHost: api.x\r\nOrigin: https://api.x\r\n\
Referer: https://api.x/login?x=1\r\nAccept-Encoding: gzip\r\n\r\n"
        );
        // Another page's origin is not ours to rename.
        let other =
            b"GET / HTTP/1.1\r\nHost: localhost:8080\r\nOrigin: http://localhost:9000\r\n\r\n";
        let (sent, _) = answer(&spec("https://api.x"), other, b"", 1);
        assert!(sent.contains("Origin: http://localhost:9000\r\n"));

        let mut off = spec("https://api.x");
        off.rewrite_host = false;
        let (sent, _) = answer(&off, req, b"", 1);
        assert_eq!(sent, String::from_utf8_lossy(req));
    }

    #[test]
    fn redirects_and_cookies_come_back_to_the_local_origin() {
        let req = b"GET / HTTP/1.1\r\nHost: 127.0.0.1:8080\r\n\r\n";
        let res = b"HTTP/1.1 302 Found\r\nLocation: https://api.x/next\r\n\
Content-Location: https://api.x:443/a\r\nRefresh: 0; url=https://api.x/b\r\n\
Link: <https://cdn.x/s.css>; rel=preload\r\nAccess-Control-Allow-Origin: https://api.x\r\n\
Set-Cookie: sid=1; Path=/; Domain=.api.x; Secure\r\nContent-Length: 3\r\n\r\nabc";
        for step in [1, 5, res.len()] {
            let (_, got) = answer(&spec("https://api.x"), req, res, step);
            assert_eq!(
                got,
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:8080/next\r\n\
Content-Location: http://127.0.0.1:8080/a\r\nRefresh: 0; url=http://127.0.0.1:8080/b\r\n\
Link: <https://cdn.x/s.css>; rel=preload\r\nAccess-Control-Allow-Origin: http://127.0.0.1:8080\r\n\
Set-Cookie: sid=1; Path=/; Secure\r\nContent-Length: 3\r\n\r\nabc"
            );
        }
        // Another port, another host: left alone.
        let other =
            b"HTTP/1.1 301 Moved\r\nLocation: https://api.x:8443/\r\nContent-Length: 0\r\n\r\n";
        let (_, got) = answer(&spec("https://api.x"), req, other, 1);
        assert_eq!(got, String::from_utf8_lossy(other));
        let longer =
            b"HTTP/1.1 301 Moved\r\nLocation: https://api.x.evil/\r\nContent-Length: 0\r\n\r\n";
        let (_, got) = answer(&spec("https://api.x"), req, longer, 1);
        assert_eq!(got, String::from_utf8_lossy(longer));
    }

    fn rewriting(url: &str) -> TunnelSpec {
        let mut s = spec(url);
        s.rewrite_content = true;
        s
    }

    #[test]
    fn a_matching_body_is_rewritten_with_a_fresh_length() {
        let req = b"GET / HTTP/1.1\r\nHost: localhost:8080\r\nAccept-Encoding: gzip, br\r\n\r\n";
        let body = r#"<a href="https://api.x/p">x</a><script>u="https:\/\/api.x\/q"</script>https://api.xy/"#;
        let res = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let want_body = r#"<a href="http://localhost:8080/p">x</a><script>u="http:\/\/localhost:8080\/q"</script>https://api.xy/"#;
        for step in [1, 7, res.len()] {
            let (sent, got) = answer(&rewriting("https://api.x"), req, res.as_bytes(), step);
            assert_eq!(
                sent,
                "GET / HTTP/1.1\r\nHost: api.x\r\nAccept-Encoding: identity\r\n\r\n"
            );
            assert_eq!(
                got,
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\r\n{want_body}",
                    want_body.len()
                )
            );
        }
    }

    #[test]
    fn a_chunked_or_unframed_body_is_decoded_then_rewritten() {
        let req = b"GET / HTTP/1.1\r\nHost: localhost:8080\r\n\r\n";
        let chunked = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n\
6\r\n{\"u\":\"\r\nf\r\nhttp://web:3000\r\n3\r\n/\"}\r\n0\r\nX-T: 1\r\n\r\n\
HTTP/1.1 204 No Content\r\n\r\n";
        let want_body = "{\"u\":\"http://localhost:8080/\"}";
        for step in [1, 3, chunked.len()] {
            let (_, got) = answer(&rewriting("http://web:3000"), req, chunked, step);
            assert_eq!(
                got,
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{want_body}\
HTTP/1.1 204 No Content\r\n\r\n",
                    want_body.len()
                )
            );
        }
        let unframed =
            b"HTTP/1.0 200 OK\r\nContent-Type: text/css\r\n\r\nurl(http://web:3000/a.png)";
        let (_, got) = answer(&rewriting("http://web:3000"), req, unframed, 4);
        assert_eq!(
            got,
            "HTTP/1.0 200 OK\r\nContent-Type: text/css\r\nContent-Length: 32\r\n\r\nurl(http://localhost:8080/a.png)"
        );
    }

    #[test]
    fn other_types_encoded_bodies_and_head_answers_pass_untouched() {
        let req = b"GET / HTTP/1.1\r\nHost: localhost:8080\r\n\r\n";
        for res in [
            &b"HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: 13\r\n\r\nhttp://web:80"[..],
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Encoding: gzip\r\nContent-Length: 13\r\n\r\nhttp://web:80",
        ] {
            let (_, got) = answer(&rewriting("http://web"), req, res, 3);
            assert_eq!(got, String::from_utf8_lossy(res));
        }
        // A HEAD's answer has a length and no body: the next response follows at once.
        let head =
            b"HEAD / HTTP/1.1\r\nHost: localhost\r\n\r\nGET / HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let res = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 9\r\n\r\n\
HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 10\r\n\r\nhttp://web";
        let (_, got) = answer(&rewriting("http://web"), head, res, 2);
        assert!(
            got.ends_with("Content-Length: 16\r\n\r\nhttp://localhost"),
            "{got}"
        );
        // An event stream is never held, whatever the patterns say.
        let mut sse = rewriting("http://web");
        sse.content_types = vec!["*".into()];
        let events =
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\ndata: http://web\n\n";
        let (_, got) = answer(&sse, req, events, 6);
        assert_eq!(got, String::from_utf8_lossy(events));
        // A body past the cap goes out as it came, unrewritten.
        let big = format!("http://web{}", "x".repeat(CAPTURE_MAX));
        let res = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{big}",
            big.len()
        );
        let (_, got) = answer(&rewriting("http://web"), req, res.as_bytes(), 64 * 1024);
        assert_eq!(got, res);
        // A switch of protocols turns the answer raw.
        let ws =
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n\x81\x02http://web";
        let (_, mut down) = rewriters(Rewrite::new(&rewriting("http://web")));
        let mut got = Vec::new();
        for piece in ws.chunks(5) {
            down.feed(piece, &mut got);
        }
        down.finish(&mut got);
        assert_eq!(got, ws);
    }

    #[test]
    fn content_type_patterns_glob() {
        assert!(glob(b"text/*", b"text/html"));
        assert!(glob(b"application/js*", b"application/json"));
        assert!(glob(b"application/*+json", b"application/ld+json"));
        assert!(!glob(b"application/*+json", b"application/json"));
        assert!(!glob(b"text/html", b"text/htmlx"));
        let mut s = rewriting("http://web");
        s.content_types = vec![" TEXT/* ".into(), String::new()];
        let r = Rewrite::new(&s);
        assert!(r.rewrites_content("text/plain; charset=utf-8"));
        assert!(!r.rewrites_content("application/json"));
        assert!(!r.rewrites_content(""));
    }

    #[test]
    fn only_a_loopback_host_passes() {
        for ok in [
            "localhost",
            "LocalHost:8080",
            "localhost.",
            "127.0.0.1",
            "127.0.0.1:1",
            "[::1]",
            "[::1]:443",
        ] {
            assert!(is_loopback_host(ok.as_bytes()), "{ok}");
        }
        for bad in [
            "evil.example",
            "localhost.evil.example",
            "127.0.0.1.nip.io",
            "127.0.0.2",
            "[::2]",
            "localhost:x",
            "localhost:",
            "",
        ] {
            assert!(!is_loopback_host(bad.as_bytes()), "{bad}");
        }
        let mut r = requests(&spec("http://h"));
        let e = r
            .feed(
                b"GET / HTTP/1.1\r\nHost: rebound.example:8080\r\n\r\n",
                &mut Vec::new(),
            )
            .unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn a_chunk_size_that_would_overflow_is_refused() {
        let mut r = requests(&spec("http://h"));
        let e = r
            .feed(
                b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffffffff\r\n",
                &mut Vec::new(),
            )
            .unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn garbage_is_refused() {
        let mut r = requests(&spec("http://h"));
        assert!(
            r.feed(b"\x16\x03\x01 not http\r\n\r\n", &mut Vec::new())
                .is_err()
        );
    }
}
