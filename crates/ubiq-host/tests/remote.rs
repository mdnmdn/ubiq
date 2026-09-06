//! The remote listener, against a real TCP socket: no coordinator, just a `Hub` and a fake
//! `HostEnd` reader, so these tests are about the transport, not about what any message means.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use ubiq_host::remote;
use ubiq_proto::bus::{self, FromClient};
use ubiq_proto::ids::PaneId;
use ubiq_proto::messages::Message;
use ubiq_proto::wire;

/// Generous: a loopback connection on a loaded CI box, not a real network.
const PATIENCE: Duration = Duration::from_secs(5);

fn start() -> (remote::Serving, bus::Hub, bus::HostEnd) {
    let (hub, host) = bus::hub();
    let serving =
        remote::serve(hub.clone(), "127.0.0.1:0".parse().unwrap()).expect("the listener to bind");
    (serving, hub, host)
}

fn connect(addr: std::net::SocketAddr) -> TcpStream {
    let stream = TcpStream::connect(addr).expect("connect");
    stream.set_read_timeout(Some(PATIENCE)).unwrap();
    stream.set_write_timeout(Some(PATIENCE)).unwrap();
    stream
}

fn send_attach(stream: &mut TcpStream, token: &str) {
    let request =
        format!("GET /attach?token={token} HTTP/1.1\r\nHost: localhost\r\nUpgrade: ubiq\r\n\r\n");
    stream.write_all(request.as_bytes()).unwrap();
}

/// Read a full HTTP response's header block — up to and including the blank line that ends it —
/// and return just the status line. Anything left unread after that blank line is the caller's,
/// which matters for the 101 case: the socket becomes wire frames right after it.
fn read_status_line(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = stream.read(&mut byte).unwrap();
        assert!(n > 0, "connection closed before a full response arrived");
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&buf);
    text.lines()
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_string()
}

fn read_full_response(stream: &mut TcpStream) -> String {
    let mut body = String::new();
    stream.read_to_string(&mut body).ok();
    body
}

#[test]
fn a_correct_token_attaches_and_a_frame_reaches_the_coordinator_side_with_a_reply_back() {
    let (serving, _hub, host) = start();
    let mut stream = connect(serving.addr);
    send_attach(&mut stream, &serving.token);

    let status = read_status_line(&mut stream);
    assert!(status.contains("101"), "expected 101, got {status}");

    // The fake coordinator: read whatever `Connected` and `Said` arrive, and answer the pane
    // message with something recognisable.
    let connected = host.recv_timeout(PATIENCE).expect("Connected");
    let client_id = match connected {
        FromClient::Connected(id) => id,
        other => panic!("expected Connected first, got {other:?}"),
    };

    let pane_id = PaneId::generate();
    wire::write_frame(&mut stream, &Message::Focus { pane_id }).unwrap();

    match host.recv_timeout(PATIENCE).expect("Said") {
        FromClient::Said { client, message } => {
            assert_eq!(client, client_id);
            match message {
                Message::Focus { pane_id: got } => assert_eq!(got, pane_id),
                other => panic!("expected Focus, got {other:?}"),
            }
        }
        other => panic!("expected Said, got {other:?}"),
    }

    // A reply travels the other way over the same socket.
    host.send(
        ubiq_proto::bus::To::Client(client_id),
        Message::PaneExited { pane_id, code: 0 },
    );
    let reply = wire::read_frame(&mut stream).expect("a reply frame");
    match reply {
        Message::PaneExited { pane_id: got, code } => {
            assert_eq!(got, pane_id);
            assert_eq!(code, 0);
        }
        other => panic!("expected PaneExited, got {other:?}"),
    }
}

#[test]
fn a_wrong_token_gets_401_and_registers_no_client() {
    let (serving, _hub, host) = start();
    let mut stream = connect(serving.addr);
    send_attach(&mut stream, "not-the-token");

    let status = read_status_line(&mut stream);
    assert!(status.contains("401"), "expected 401, got {status}");

    wait_for(|| host.attached().is_empty());
    assert!(host.attached().is_empty());
}

#[test]
fn a_missing_token_gets_401() {
    let (serving, _hub, _host) = start();
    let mut stream = connect(serving.addr);
    stream
        .write_all(b"GET /attach? HTTP/1.1\r\nHost: localhost\r\nUpgrade: ubiq\r\n\r\n")
        .unwrap();
    let status = read_status_line(&mut stream);
    assert!(status.contains("401"), "expected 401, got {status}");
}

#[test]
fn get_root_answers_200_with_no_token_in_the_body() {
    let (serving, _hub, _host) = start();
    let mut stream = connect(serving.addr);
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let status = read_status_line(&mut stream);
    assert!(status.contains("200"), "expected 200, got {status}");
    let body = read_full_response(&mut stream);
    assert!(
        !body.contains(&serving.token),
        "the root page must never leak the token"
    );
}

#[test]
fn an_oversized_header_block_is_rejected_rather_than_buffered_without_limit() {
    let (serving, _hub, _host) = start();
    let mut stream = connect(serving.addr);
    // A single header line well past the 8 KiB cap, with no terminator in sight.
    let mut request = String::from("GET /attach?token=x HTTP/1.1\r\n");
    request.push_str("X-Filler: ");
    request.push_str(&"a".repeat(16 * 1024));
    stream.write_all(request.as_bytes()).unwrap();
    let status = read_status_line(&mut stream);
    assert!(
        status.contains("400"),
        "expected the oversized header to be refused, got {status}"
    );
}

#[test]
fn dropping_the_socket_deregisters_the_client() {
    let (serving, _hub, host) = start();
    let mut stream = connect(serving.addr);
    send_attach(&mut stream, &serving.token);
    let _status = read_status_line(&mut stream);

    let _ = host.recv_timeout(PATIENCE).expect("Connected");
    wait_for(|| host.attached().len() == 1);
    assert_eq!(host.attached().len(), 1);

    drop(stream);

    // The host side sees `Gone` too, though the test only needs the routing table to clear.
    wait_for(|| host.attached().is_empty());
    assert!(host.attached().is_empty());
}

/// Poll a condition instead of assuming any particular latency for the accept and reader/writer
/// threads to settle — those run on their own schedule.
fn wait_for(mut condition: impl FnMut() -> bool) {
    let start = std::time::Instant::now();
    while !condition() {
        if start.elapsed() > PATIENCE {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
