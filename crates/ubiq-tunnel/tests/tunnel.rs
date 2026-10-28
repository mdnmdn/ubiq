//! End to end over a loopback socket pair standing in for the carrier's channel: a window's
//! [`TunnelManager`] on one end, a drone's [`serve`] on the other, real TCP servers behind it.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use ubiq_proto::settings::{TunnelKind, TunnelSpec};
use ubiq_proto::tunnel::TUNNEL_STREAM_WINDOW;
use ubiq_tunnel::{AllowList, Mux, Recv, TunnelManager, serve};

/// A connected socket pair, the duplex a carrier would hand over.
fn duplex() -> (TcpStream, TcpStream) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let a = TcpStream::connect(l.local_addr().unwrap()).unwrap();
    let (b, _) = l.accept().unwrap();
    (a, b)
}

fn closer(s: &TcpStream) -> Option<Box<dyn FnOnce() + Send>> {
    let s = s.try_clone().unwrap();
    Some(Box::new(move || {
        let _ = s.shutdown(Shutdown::Both);
    }))
}

/// A window mux and a drone mux over one duplex, without serving.
fn link_pair(rate_bps: Option<u64>) -> (Mux, Mux) {
    let (a, b) = duplex();
    let drone = thread::spawn(move || {
        let c = closer(&b);
        Mux::drone(b.try_clone().unwrap(), b, c).unwrap()
    });
    let c = closer(&a);
    let window = Mux::window(a.try_clone().unwrap(), a, rate_bps, c).unwrap();
    (window, drone.join().unwrap())
}

/// A window mux whose drone serves with `allow`; the drone mux comes back so a test can drop it.
fn served(rate_bps: Option<u64>, allow: AllowList) -> (Mux, Mux) {
    let (window, drone) = link_pair(rate_bps);
    let d = drone.clone();
    thread::spawn(move || serve(&d, &allow));
    (window, drone)
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn spec(kind: TunnelKind, target: u16) -> TunnelSpec {
    TunnelSpec {
        id: "t1".into(),
        kind,
        local_port: free_port(),
        host: "127.0.0.1".into(),
        port: target,
        upstream_tls: false,
        host_rewrite: None,
        insecure: false,
        enabled: true,
    }
}

fn echo_server() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    thread::spawn(move || {
        for s in l.incoming() {
            let mut s = s.unwrap();
            thread::spawn(move || {
                let mut r = s.try_clone().unwrap();
                let _ = std::io::copy(&mut r, &mut s);
                let _ = s.shutdown(Shutdown::Write);
            });
        }
    });
    port
}

fn connect(port: u16) -> TcpStream {
    let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s
}

/// Write `data`, half-close, read everything back.
fn round_trip(port: u16, data: &[u8]) -> Vec<u8> {
    let mut s = connect(port);
    let mut w = s.try_clone().unwrap();
    let data = data.to_vec();
    let writer = thread::spawn(move || {
        w.write_all(&data).unwrap();
        w.shutdown(Shutdown::Write).unwrap();
    });
    let mut back = Vec::new();
    s.read_to_end(&mut back).unwrap();
    writer.join().unwrap();
    back
}

#[test]
fn tcp_echoes_through_the_tunnel_past_many_windows() {
    let (window, _drone) = served(None, AllowList::any());
    let spec = spec(TunnelKind::Tcp, echo_server());
    let manager = TunnelManager::start(std::slice::from_ref(&spec), Some(window));
    let data: Vec<u8> = (0..1_000_000u32).map(|i| (i % 251) as u8).collect();
    assert_eq!(round_trip(spec.local_port, &data), data);

    let status = &manager.status()[0];
    assert!(status.listening && status.linked);
    assert_eq!(status.conns, 1);
    assert_eq!(status.bytes_up, data.len() as u64);
    assert_eq!(status.bytes_down, data.len() as u64);
}

#[test]
fn credit_holds_a_fast_sender_to_its_window() {
    let (window, drone) = link_pair(None);
    let incoming = drone.incoming().unwrap();
    // The drone accepts five streams and reads none of them yet.
    let acceptor = thread::spawn(move || {
        (0..5)
            .map(|_| incoming.recv().unwrap().accept())
            .collect::<Vec<_>>()
    });
    let sent = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut senders = Vec::new();
    for _ in 0..5 {
        let stream = window.open("anywhere", 1).unwrap();
        let sent = sent.clone();
        senders.push(thread::spawn(move || {
            let chunk = [7u8; 4096];
            for _ in 0..64 {
                stream.send(&chunk).unwrap();
                sent.fetch_add(chunk.len(), std::sync::atomic::Ordering::SeqCst);
            }
            // Dropped open, a stream resets; hand it back instead.
            stream
        }));
    }
    let drone_streams = acceptor.join().unwrap();
    thread::sleep(Duration::from_millis(300));
    let stuck = sent.load(std::sync::atomic::Ordering::SeqCst);
    // Each stream fills its own window, and no more.
    assert_eq!(
        stuck,
        5 * TUNNEL_STREAM_WINDOW as usize,
        "the senders stopped at {stuck} bytes"
    );

    // Draining and acking releases every sender.
    let mut drained = 0;
    let readers: Vec<_> = drone_streams
        .into_iter()
        .map(|s| {
            thread::spawn(move || {
                let mut n = 0;
                while n < 64 * 4096 {
                    match s.recv() {
                        Recv::Data(b) => {
                            n += b.len();
                            s.ack(b.len());
                        }
                        other => panic!("{other:?}"),
                    }
                }
                n
            })
        })
        .collect();
    for r in readers {
        drained += r.join().unwrap();
    }
    let _streams: Vec<_> = senders.into_iter().map(|s| s.join().unwrap()).collect();
    assert_eq!(drained, 5 * 64 * 4096);
}

#[test]
fn stalled_streams_never_block_another() {
    let (window, drone) = link_pair(None);
    let incoming = drone.incoming().unwrap();
    // Eight streams whose consumer never reads — more windows than any link-wide cap would allow.
    let acceptor = {
        let incoming = incoming.clone();
        thread::spawn(move || {
            (0..8)
                .map(|_| incoming.recv().unwrap().accept())
                .collect::<Vec<_>>()
        })
    };
    let stalled: Vec<_> = (0..8).map(|_| window.open("stalled", 1).unwrap()).collect();
    let held = acceptor.join().unwrap();
    let fillers: Vec<_> = stalled
        .into_iter()
        .map(|s| {
            thread::spawn(move || {
                // Fills the window, then blocks for good; the reset at the end frees it.
                let _ = s.send(&vec![0u8; 2 * TUNNEL_STREAM_WINDOW as usize]);
            })
        })
        .collect();
    thread::sleep(Duration::from_millis(200));

    // A ninth stream still moves a megabyte.
    let acceptor = thread::spawn(move || incoming.recv().unwrap().accept());
    let live = window.open("live", 1).unwrap();
    let far = acceptor.join().unwrap();
    let total = 1_000_000;
    let reader = thread::spawn(move || {
        let mut n = 0;
        while n < total {
            match far.recv() {
                Recv::Data(b) => {
                    n += b.len();
                    far.ack(b.len());
                }
                other => panic!("{other:?}"),
            }
        }
        far
    });
    let started = Instant::now();
    live.send(&vec![9u8; total]).unwrap();
    let _far = reader.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));

    drop(held);
    for f in fillers {
        f.join().unwrap();
    }
}

/// A server that records every byte of one connection and answers nothing.
fn recording_server() -> (u16, std::sync::mpsc::Receiver<Vec<u8>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut all = Vec::new();
        let _ = s.read_to_end(&mut all);
        tx.send(all).unwrap();
    });
    (port, rx)
}

#[test]
fn http_rewrites_host_on_pipelined_and_chunked_requests() {
    let (window, _drone) = served(None, AllowList::any());
    let (target, recorded) = recording_server();
    let mut spec = spec(TunnelKind::Http, target);
    spec.host_rewrite = Some("api.example.com".into());
    let _manager = TunnelManager::start(std::slice::from_ref(&spec), Some(window));

    let sent = b"POST /a HTTP/1.1\r\nHost: localhost\r\nContent-Length: 12\r\n\r\nHost: inside\
POST /b HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n6\r\nHost: \r\n0\r\n\r\n\
GET /c HTTP/1.1\r\nUser-Agent: t\r\n\r\n";
    let mut s = connect(spec.local_port);
    for piece in sent.chunks(9) {
        s.write_all(piece).unwrap();
    }
    s.shutdown(Shutdown::Write).unwrap();
    let got = recorded.recv_timeout(Duration::from_secs(10)).unwrap();
    let want = b"POST /a HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: 12\r\n\r\nHost: inside\
POST /b HTTP/1.1\r\nHost: api.example.com\r\nTransfer-Encoding: chunked\r\n\r\n6\r\nHost: \r\n0\r\n\r\n\
GET /c HTTP/1.1\r\nUser-Agent: t\r\nHost: api.example.com\r\n\r\n";
    assert_eq!(String::from_utf8_lossy(&got), String::from_utf8_lossy(want));
}

#[test]
fn a_rebound_host_is_a_403_and_reaches_nothing() {
    let (window, _drone) = served(None, AllowList::any());
    let (target, recorded) = recording_server();
    let spec = spec(TunnelKind::Http, target);
    let manager = TunnelManager::start(std::slice::from_ref(&spec), Some(window));

    let answer = round_trip(
        spec.local_port,
        b"GET /admin HTTP/1.1\r\nHost: rebound.example:8080\r\n\r\n",
    );
    let answer = String::from_utf8_lossy(&answer);
    assert!(answer.starts_with("HTTP/1.1 403"), "{answer}");
    assert!(answer.contains("only localhost"), "{answer}");
    let got = recorded.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(got.is_empty(), "{}", String::from_utf8_lossy(&got));
    assert!(
        manager.status()[0]
            .last_error
            .as_deref()
            .unwrap()
            .contains("only localhost")
    );
}

#[test]
fn a_stopped_manager_frees_its_ports_at_once() {
    let spec = spec(TunnelKind::Tcp, free_port());
    for _ in 0..5 {
        let manager = TunnelManager::start(std::slice::from_ref(&spec), None);
        assert!(manager.status()[0].listening, "{:?}", manager.status());
        manager.stop();
        drop(manager);
    }
}

#[test]
fn a_refused_dial_is_a_502_and_a_last_error() {
    let closed = free_port();
    let (window, _drone) = served(
        None,
        AllowList::parse([format!("127.0.0.1:{closed}")]).unwrap(),
    );
    let http = spec(TunnelKind::Http, closed);
    let mut tcp = spec(TunnelKind::Tcp, closed);
    tcp.id = "t2".into();
    let mut blocked = spec(TunnelKind::Tcp, 2);
    blocked.id = "t3".into();
    let manager = TunnelManager::start(&[http.clone(), tcp.clone(), blocked.clone()], Some(window));

    let answer = round_trip(http.local_port, b"GET / HTTP/1.1\r\nHost: x\r\n\r\n");
    let answer = String::from_utf8_lossy(&answer);
    assert!(answer.starts_with("HTTP/1.1 502"), "{answer}");
    assert!(answer.contains("was refused"), "{answer}");

    assert!(round_trip(tcp.local_port, b"hi").is_empty());
    assert!(round_trip(blocked.local_port, b"hi").is_empty());
    let status = manager.status();
    assert!(status[1].last_error.as_deref().unwrap().contains("refused"));
    assert!(
        status[2]
            .last_error
            .as_deref()
            .unwrap()
            .contains("allow list")
    );
}

#[test]
fn a_dropped_link_resets_streams_refuses_new_ones_and_a_new_link_heals() {
    let (window, drone) = served(None, AllowList::any());
    let spec = spec(TunnelKind::Tcp, echo_server());
    let manager = TunnelManager::start(std::slice::from_ref(&spec), Some(window));

    let mut open = connect(spec.local_port);
    open.write_all(b"ping").unwrap();
    let mut buf = [0u8; 4];
    open.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"ping");

    drone.close();
    let deadline = Instant::now() + Duration::from_secs(5);
    while manager.linked() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(!manager.linked());
    // The stream that was open ends.
    let mut rest = Vec::new();
    let _ = open.read_to_end(&mut rest);
    assert!(rest.is_empty());
    // A new connection is turned away, with a reason.
    assert!(round_trip(spec.local_port, b"hi").is_empty());
    let status = &manager.status()[0];
    assert!(status.listening && !status.linked);
    assert!(
        status
            .last_error
            .as_deref()
            .unwrap()
            .contains("link is down")
    );

    let (window, _drone) = served(None, AllowList::any());
    manager.attach(window);
    assert_eq!(round_trip(spec.local_port, b"again"), b"again");
}

#[test]
fn the_rate_ceiling_holds_both_ways() {
    // 400 KB/s, honoured by the drone too through the hello.
    let (window, _drone) = served(Some(400_000), AllowList::any());
    assert_eq!(window.rate_bps(), Some(400_000));
    let spec = spec(TunnelKind::Tcp, echo_server());
    let _manager = TunnelManager::start(std::slice::from_ref(&spec), Some(window));
    let data = vec![1u8; 400_000];
    let started = Instant::now();
    assert_eq!(round_trip(spec.local_port, &data).len(), data.len());
    // Less one burst each way, about a second.
    assert!(
        started.elapsed() > Duration::from_millis(800),
        "{:?}",
        started.elapsed()
    );
}
