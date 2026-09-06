//! Phase 3b: reach a remote host from the UI, and the client half of the wire it once it does.
//!
//! **Why this lives here and not as its own file elsewhere in `app/`.** The transport section
//! below (`dial`, the handshake, the two pump threads) has exactly one caller: `try_connect`, a
//! few lines down in the same file. Splitting it out would trade one well-labelled module for two
//! small ones that only ever call each other — the doc comments below are the seam instead. If a
//! second transport (a saved-host reconnect, say) shows up in a later phase, that is the point to
//! actually split it.
//!
//! **The transport is deliberately dumb.** Exactly like `ubiq_host::remote`'s listener, this knows
//! nothing about any message family — it is attach, pump frames onto a `Client`, pump the
//! `Client`'s outbox onto the socket, and stop when either side does. Read `ubiq_host::remote`
//! alongside this: the two are mirror images of the same handshake, one binding and one dialing.
//!
//! **Never on the GPUI thread.** Every step in [`dial`] is a blocking syscall — a DNS lookup, a
//! TCP connect, a byte-at-a-time header read. [`AppState::try_connect_remote`] runs it inside
//! `cx.background_spawn`, exactly the way `chat.rs` farms out a diagram render, and reads the
//! answer back through `cx.spawn` updating the entity. A modal that called this straight from a
//! click handler would freeze the whole window for as long as a dead address takes to time out.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::thread;
use std::time::Duration;

use ubiq_proto::bus::{self, Client, FromClient};
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
            ConnectFailure::Unreachable(reason) => write!(f, "could not reach that address: {reason}"),
            ConnectFailure::TokenRejected => write!(f, "that host rejected the token"),
            ConnectFailure::Refused(reason) => write!(f, "that host did not answer as a Ubiq host: {reason}"),
            ConnectFailure::Io(reason) => write!(f, "the connection failed: {reason}"),
            ConnectFailure::Untypable(part) => {
                write!(f, "that {part} contains a character an address cannot carry")
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
/// budget before a newline"; `Ok(Some(0))` is a clean EOF with nothing read.
///
/// One byte per syscall, deliberately not a `BufReader`: exactly the trade `ubiq_host::remote`'s
/// own `read_capped_line` makes, and for the same reason — a buffered reader can pull well past
/// the header block's end in one underlying read, and everything past the blank line that ends
/// the response is the first wire frame once the upgrade lands. Swallowing any of it into a buffer
/// this function is about to drop would desync the two ends from the very first message.
fn read_capped_line(stream: &mut TcpStream, line: &mut String, budget: usize) -> io::Result<Option<usize>> {
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
fn read_response_status(stream: &mut TcpStream) -> Result<u16, ConnectFailure> {
    let mut total = 0usize;
    let mut status_line: Option<String> = None;
    loop {
        let mut line = String::new();
        let read = read_capped_line(stream, &mut line, MAX_HEADER - total)
            .map_err(|error| ConnectFailure::Io(error.to_string()))?;
        match read {
            None => return Err(ConnectFailure::Refused("response header too large".to_string())),
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
    let status_line = status_line.ok_or_else(|| ConnectFailure::Refused("empty response".to_string()))?;
    parse_status_code(&status_line).ok_or(ConnectFailure::Refused(status_line))
}

/// Dial one remote host and, on success, hand back the `Client` a `Bus` registers.
///
/// Entirely blocking, on purpose — see the module doc comment on why the caller must run this off
/// the GPUI thread. `address` is `host:port`; callers pass it through
/// [`crate::state::remote::with_default_port`] first so a bare host still resolves.
fn dial(address: &str, token: &str) -> Result<Client, ConnectFailure> {
    // Before the socket, not after: a request that must not be sent is one this never builds.
    if !is_typable(address) {
        return Err(ConnectFailure::Untypable("address"));
    }
    if !is_typable(token) {
        return Err(ConnectFailure::Untypable("token"));
    }

    let socket_addr = address
        .to_socket_addrs()
        .map_err(|error| ConnectFailure::Unreachable(error.to_string()))?
        .next()
        .ok_or_else(|| ConnectFailure::Unreachable(format!("no address found for {address}")))?;

    let mut stream = TcpStream::connect_timeout(&socket_addr, CONNECT_TIMEOUT)
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

    stream
        .write_all(attach_request(address, token).as_bytes())
        .map_err(|error| ConnectFailure::Io(error.to_string()))?;

    match read_response_status(&mut stream)? {
        101 => {}
        401 => return Err(ConnectFailure::TokenRejected),
        other => return Err(ConnectFailure::Refused(format!("host answered {other}"))),
    }

    stream
        .set_read_timeout(None)
        .map_err(|error| ConnectFailure::Io(error.to_string()))?;

    let (client, detached) = bus::detached();
    spawn_pump(stream, detached);
    Ok(client)
}

/// Shuttle frames between the socket and a [`bus::Detached`] until either side gives up.
///
/// The shape is `ubiq_host::remote::pump`'s, mirrored: there, one `Client` came from a `Hub` and
/// the socket was accepted; here, the `Client` is [`bus::detached`]'s and the socket was dialed.
/// Everything past that — clone the stream for the two directions, poll `said()` so the writer
/// notices a dead reader without a blocking `recv` masking it, shut the socket down from whichever
/// side notices first — is identical for the same reason: a session, once the handshake behind it
/// is done, does not care which end opened the connection.
fn spawn_pump(stream: TcpStream, detached: bus::Detached) {
    // Flume's `Sender`/`Receiver` clone by sharing the same queue, so the two threads below can
    // each own a handle with no `Arc` and no need to keep `detached` itself alive.
    let said = detached.said().clone();
    let deliver = detached.deliver().clone();

    let (mut reader_stream, mut writer_stream) = match stream.try_clone() {
        Ok(clone) => (clone, stream),
        // `try_clone` fails only on fd exhaustion or a kernel refusing to duplicate the handle —
        // rare enough that surfacing a second failure path here is not worth it. The `Client`
        // still exists; it just never hears from the host, which is the same outcome a dead
        // socket produces on its own.
        Err(_) => return,
    };

    let writer_said = said;
    thread::Builder::new()
        .name("ubiq-remote-client-writer".to_string())
        .spawn(move || {
            loop {
                match writer_said.recv_timeout(STOP_POLL) {
                    Ok(FromClient::Said { message, .. }) => {
                        if wire::write_frame(&mut writer_stream, &message).is_err() {
                            break;
                        }
                    }
                    // `Connected` never fires for a detached client — only `Hub::connect` sends
                    // it — and `Gone` is the local `Client` being dropped, an intentional
                    // teardown rather than a failure to report.
                    Ok(FromClient::Connected(_)) => {}
                    Ok(FromClient::Gone(_)) => break,
                    Err(flume::RecvTimeoutError::Timeout) => {}
                    Err(flume::RecvTimeoutError::Disconnected) => break,
                }
            }
            let _ = writer_stream.shutdown(Shutdown::Both);
        })
        .expect("the remote client's writer thread");

    thread::Builder::new()
        .name("ubiq-remote-client-reader".to_string())
        .spawn(move || {
            while let Ok(message) = wire::read_frame(&mut reader_stream) {
                if deliver.send(message).is_err() {
                    break;
                }
            }
            let _ = reader_stream.shutdown(Shutdown::Both);
        })
        .expect("the remote client's reader thread");
}

// ── The modal ───────────────────────────────────────────────────────────────

impl AppState {
    /// Raise the connect modal, with both fields empty.
    pub fn open_remote_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_remote_connect_inputs(window, cx);
        self.workbench.remote_connect = Some(RemoteConnectState::default());
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
    /// alongside it and rewrites the address field down to just the address — the one paste a
    /// user reaching for this modal actually has in their clipboard, made to work without asking
    /// them to split it themselves first.
    pub(super) fn apply_remote_address_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.remote_address_input.read(cx).value().to_string();
        let parsed = crate::state::remote::parse_connection_string(&typed);
        if parsed.address == typed.trim() && parsed.token.is_none() {
            // An ordinary keystroke, or a paste of a bare address: nothing to rewrite.
            return;
        }
        let Some(token) = parsed.token else {
            return;
        };
        self.remote_address_input.update(cx, |state, cx| {
            state.set_value(parsed.address, window, cx);
        });
        self.remote_token_input.update(cx, |state, cx| {
            state.set_value(token, window, cx);
        });
    }

    /// Start a dial. Reads both fields, mints the [`AttemptId`] the answer must carry to be
    /// believed, and runs [`dial`] on the background executor so this call returns immediately —
    /// the modal redraws itself in the `Connecting` step while the socket work happens elsewhere.
    pub fn try_connect_remote(&mut self, cx: &mut Context<Self>) {
        let address = self.remote_address_input.read(cx).value().trim().to_string();
        let token = self.remote_token_input.read(cx).value().trim().to_string();
        if address.is_empty() || token.is_empty() {
            return;
        }
        let dial_address = with_default_port(&address);

        let attempt = AttemptId::generate();
        if let Some(state) = &mut self.workbench.remote_connect {
            state.step = RemoteConnectStep::Connecting { attempt };
        }
        cx.notify();

        let outcome = cx.background_spawn(async move { dial(&dial_address, &token) });
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let outcome = outcome.await;
            let _ = this.update(cx, |this, cx| this.land_remote_connect(attempt, outcome, cx));
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

        match outcome {
            Ok(client) => {
                let label = self.remote_address_input.read(cx).value().to_string();
                self.attach_remote(client, label.clone(), cx);
                if let Some(state) = &mut self.workbench.remote_connect {
                    state.step = RemoteConnectStep::Connected { label };
                }
            }
            Err(failure) => {
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
    /// `label` is the address the user reached it by — the only name this connection has until
    /// Phase 5's saved-hosts list gives it a better one.
    fn attach_remote(&mut self, client: Client, label: String, cx: &mut Context<Self>) {
        let (host_id, from_host) = self.bus.register_remote(client, label);
        Self::route_host(HostRef::Remote(host_id), from_host, cx);
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
        assert_eq!(parse_status_code("HTTP/1.1 101 Switching Protocols"), Some(101));
        assert_eq!(parse_status_code("HTTP/1.1 401 Unauthorized"), Some(401));
    }

    #[test]
    fn a_status_line_with_no_recognisable_code_parses_to_none() {
        assert_eq!(parse_status_code("not a status line"), None);
        assert_eq!(parse_status_code(""), None);
    }
}
