//! The HTTP kind's request direction: rewrite each request's `Host`, leave every body alone.
//!
//! A streaming state machine over the bytes a browser sends. It parses a request head with
//! `httparse`, writes it back with `Host` replaced (or added), then copies the body by its own
//! framing — `Content-Length`, or `chunked` to its last-chunk and trailers — and parses the next
//! head after it, so pipelined and keep-alive requests are each rewritten. After a head that asks
//! for an upgrade (a websocket) or a `CONNECT`, the rest of the stream passes untouched.
//!
//! The response direction is never parsed: what comes back is the server's, byte for byte.

use std::io;

use httparse::Status;

/// The longest request head accepted before the stream is refused.
const HEAD_MAX: usize = 64 * 1024;
/// The longest chunk-size or trailer line accepted.
const LINE_MAX: usize = 8 * 1024;
const MAX_HEADERS: usize = 128;

enum State {
    Head,
    Body(u64),
    ChunkSize,
    /// The chunk's bytes plus its trailing CRLF.
    ChunkData(u64),
    ChunkTrailer,
    /// An upgraded or `CONNECT` stream: no more HTTP.
    Raw,
}

pub(crate) struct HostRewriter {
    host: String,
    state: State,
    buf: Vec<u8>,
}

impl HostRewriter {
    pub(crate) fn new(host: String) -> Self {
        Self {
            host,
            state: State::Head,
            buf: Vec::new(),
        }
    }

    /// Feed bytes read from the client; append what to send upstream to `out`.
    pub(crate) fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
        if self.buf.is_empty() {
            match self.state {
                State::Raw => {
                    out.extend_from_slice(input);
                    return Ok(());
                }
                State::Body(n) if n as usize >= input.len() => {
                    out.extend_from_slice(input);
                    self.state = Self::after_body(n - input.len() as u64);
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
                State::Raw => {
                    out.extend_from_slice(rest);
                    pos = self.buf.len();
                }
                State::Body(n) => {
                    let k = (n as usize).min(rest.len());
                    out.extend_from_slice(&rest[..k]);
                    pos += k;
                    self.state = Self::after_body(n - k as u64);
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
                    self.state = if size == 0 {
                        State::ChunkTrailer
                    } else {
                        State::ChunkData(size + 2)
                    };
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
                State::Head => {
                    // A stray CRLF between requests is allowed; pass it on.
                    if rest.starts_with(b"\r\n") {
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
                            self.state = write_head(&self.host, rest, &req, out)?;
                            pos += n;
                        }
                        Err(e) => return Err(invalid(&format!("not an HTTP request: {e}"))),
                    }
                }
            }
        }
        self.buf.drain(..pos);
        Ok(())
    }

    fn after_body(left: u64) -> State {
        if left == 0 {
            State::Head
        } else {
            State::Body(left)
        }
    }
}

/// Write the head with `Host` replaced, and say what follows it.
fn write_head(
    host: &str,
    raw: &[u8],
    req: &httparse::Request,
    out: &mut Vec<u8>,
) -> io::Result<State> {
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
            if !wrote_host {
                push_header(out, "Host", host.as_bytes());
                wrote_host = true;
            }
            continue;
        }
        if name.eq_ignore_ascii_case("content-length") {
            let text = std::str::from_utf8(h.value).unwrap_or("").trim();
            length = Some(
                text.parse()
                    .map_err(|_| invalid("a Content-Length that is not a number"))?,
            );
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            let text = String::from_utf8_lossy(h.value).to_ascii_lowercase();
            chunked = text.rsplit(',').next().map(str::trim) == Some("chunked");
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
    if !wrote_host {
        push_header(out, "Host", host.as_bytes());
    }
    out.extend_from_slice(b"\r\n");
    Ok(
        if req.method == Some("CONNECT") || (upgrade && connection_upgrade) {
            State::Raw
        } else if chunked {
            State::ChunkSize
        } else {
            HostRewriter::after_body(length.unwrap_or(0))
        },
    )
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

    fn run(input: &[u8], step: usize) -> Vec<u8> {
        let mut r = HostRewriter::new("api.example.com".into());
        let mut out = Vec::new();
        for piece in input.chunks(step) {
            r.feed(piece, &mut out).unwrap();
        }
        out
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
        let input = b"POST /up HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n\
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
            b"GET /ws HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n\
GET /not-a-request HTTP/1.1\r\nHost: x\r\n\r\n\x81\x05hello";
        let out = run(input, 4);
        let head = b"GET /ws HTTP/1.1\r\nHost: api.example.com\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n";
        let first = input.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        assert!(out.starts_with(head));
        assert_eq!(&out[head.len()..], &input[first..]);
    }

    #[test]
    fn garbage_is_refused() {
        let mut r = HostRewriter::new("h".into());
        assert!(
            r.feed(b"\x16\x03\x01 not http\r\n\r\n", &mut Vec::new())
                .is_err()
        );
    }
}
