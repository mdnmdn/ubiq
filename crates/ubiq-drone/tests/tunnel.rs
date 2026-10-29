//! `ubiq-drone --tunnel`, end to end over its own standard input and output — what a window's
//! second `ssh` exec speaks to (`D227`): the window's mux on the child's pipes, a stream opened
//! through it to a loopback echo server, and the bytes back.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::thread;

use ubiq_tunnel::{Mux, Recv};

/// A loopback server that echoes one connection's bytes back.
fn echo_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut buf = [0u8; 1024];
        loop {
            match socket.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => socket.write_all(&buf[..n]).unwrap(),
            }
        }
    });
    port
}

fn tunnel_drone(extra: &[&str]) -> (std::process::Child, Mux) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ubiq-drone"))
        .arg("--tunnel")
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("the drone starts");
    let stdout = child.stdout.take().unwrap();
    let stdin = child.stdin.take().unwrap();
    let mux = Mux::window(stdout, stdin, None, None).expect("the tunnel hello");
    (child, mux)
}

fn read_back(stream: &ubiq_tunnel::TunnelStream, want: usize) -> Vec<u8> {
    let mut got = Vec::new();
    while got.len() < want {
        match stream.recv() {
            Recv::Data(bytes) => {
                stream.ack(bytes.len());
                got.extend_from_slice(&bytes);
            }
            other => panic!("the stream ended early: {other:?}"),
        }
    }
    got
}

#[test]
fn the_stdio_mode_round_trips_a_stream() {
    let port = echo_server();
    let (mut child, mux) = tunnel_drone(&[]);
    let stream = mux.open("127.0.0.1", port).expect("the drone dials");
    stream.send(b"hello through the drone").unwrap();
    assert_eq!(read_back(&stream, 23), b"hello through the drone");

    // The window closing the link ends the process.
    drop(stream);
    mux.close();
    let status = child.wait().unwrap();
    assert!(status.success(), "the drone exits cleanly: {status:?}");
}

#[test]
fn the_allow_list_refuses_what_it_does_not_name() {
    let port = echo_server();
    let (mut child, mux) = tunnel_drone(&["--tunnel-allow", "localhost:1"]);
    let refused = mux.open("127.0.0.1", port);
    assert!(
        refused.as_ref().is_err_and(|e| e.contains("allow list")),
        "refused by the allow list"
    );
    mux.close();
    let _ = child.wait();
}
