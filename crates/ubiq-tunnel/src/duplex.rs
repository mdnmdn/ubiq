//! One TLS session over one TCP socket, shared by a reading thread and a writing thread.
//!
//! rustls has no `try_clone`, so both directions hold the same session behind a lock. That lock is
//! **never held across socket I/O**: the reader waits for ciphertext unlocked and takes the lock
//! only to hand it to the session and take the plaintext out; the writer takes it only to seal its
//! plaintext, and sends the records after letting go. So a writer is never stuck behind an idle
//! reader — which a tunnel's [`crate::Mux`] needs, since its writer sends an `open` while its
//! reader waits for the answer — and a reader is never stuck behind a writer the peer is not
//! draining.
//!
//! Records leave in the order they were sealed: the session queues them as they are sealed, and
//! only the send lock's holder drains that queue. Every sender goes through `send_owed`, which
//! re-checks the queue after letting the lock go, so a record queued while another held it is
//! never left behind.
//! One reader at a time: reads on clones are serialised by a lock of their own.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use parking_lot::Mutex;

/// The ciphertext a single socket read may take.
const CHUNK: usize = 16 * 1024;

/// See the module comment. Cheap to clone; every clone is the same session.
#[derive(Clone)]
pub struct TlsDuplex {
    session: Arc<Mutex<rustls::Connection>>,
    sock: Arc<TcpStream>,
    /// The reader's lock, over its own state.
    reading: Arc<Mutex<Reading>>,
    sending: Arc<Mutex<()>>,
}

/// What only the reader touches.
#[derive(Default)]
struct Reading {
    /// Ciphertext read off the socket that the session has not taken yet.
    pending: Vec<u8>,
    /// The socket read buffer, allocated on the first wait and kept.
    chunk: Vec<u8>,
}

impl TlsDuplex {
    /// Wrap a session whose handshake is done and the socket it ran on.
    pub fn new(conn: impl Into<rustls::Connection>, sock: TcpStream) -> Self {
        Self {
            session: Arc::new(Mutex::new(conn.into())),
            sock: Arc::new(sock),
            reading: Arc::new(Mutex::new(Reading::default())),
            sending: Arc::new(Mutex::new(())),
        }
    }

    /// Send whatever the session owes, until it owes nothing. Whoever holds the send lock
    /// re-checks after letting go, so a reader that found the lock taken (`wait` false, it never
    /// waits on a writer) can leave its answer — an alert, a key update — to the holder, and the
    /// answer is never stranded on an idle link.
    fn send_owed(&self, wait: bool) -> io::Result<()> {
        loop {
            {
                let _sending = if wait {
                    self.sending.lock()
                } else {
                    match self.sending.try_lock() {
                        Some(guard) => guard,
                        None => return Ok(()),
                    }
                };
                self.send_sealed()?;
            }
            if !self.session.lock().wants_write() {
                return Ok(());
            }
        }
    }

    /// The socket underneath: for its timeouts and its `shutdown`.
    pub fn socket(&self) -> &TcpStream {
        &self.sock
    }

    /// Seal what the session wants sent, then send it. The caller holds the send lock.
    fn send_sealed(&self) -> io::Result<()> {
        let mut records = Vec::new();
        {
            let mut conn = self.session.lock();
            while conn.wants_write() {
                conn.write_tls(&mut records)?;
            }
        }
        if !records.is_empty() {
            (&*self.sock).write_all(&records)?;
        }
        Ok(())
    }
}

impl Read for TlsDuplex {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut reading = self.reading.lock();
        let Reading { pending, chunk } = &mut *reading;
        loop {
            // `None`: nothing pending, so wait on the socket. `Some(processed)`: ciphertext was
            // fed to the session — send any answer it owes, then look for plaintext again.
            let fed = {
                let mut conn = self.session.lock();
                match conn.reader().read(buf) {
                    // Zero is the peer's `close_notify`.
                    Ok(n) => return Ok(n),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e),
                }
                if pending.is_empty() {
                    None
                } else {
                    let mut rest = &pending[..];
                    let taken = conn.read_tls(&mut rest)?;
                    if taken == 0 {
                        return Err(io::Error::other("the TLS session took no ciphertext"));
                    }
                    pending.drain(..taken);
                    Some(
                        conn.process_new_packets()
                            .map(drop)
                            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
                    )
                }
            };
            if let Some(processed) = fed {
                // A key update to answer, or — when processing failed — the alert rustls queued
                // for the peer, which goes out before the error does. A writer busy sending
                // carries it ([`Self::send_owed`]); waiting for it here would be waiting on the
                // peer.
                let sent = self.send_owed(false);
                processed?;
                sent?;
                continue;
            }
            if chunk.is_empty() {
                chunk.resize(CHUNK, 0);
            }
            let n = (&*self.sock).read(chunk)?;
            if n == 0 {
                return Ok(0);
            }
            pending.extend_from_slice(&chunk[..n]);
        }
    }
}

impl Write for TlsDuplex {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Sealed into the session's queue now; whoever holds the send lock drains it in order.
        let n = self.session.lock().writer().write(buf)?;
        self.send_owed(true)?;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.send_owed(true)?;
        (&*self.sock).flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pin::{Trust, client_config, fingerprint};
    use crate::{AllowList, Mux, Recv};
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    const CERT: &[u8] = include_bytes!("../testdata/cert.pem");
    const KEY: &[u8] = include_bytes!("../testdata/key.pem");

    fn echo_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for sock in listener.incoming().flatten() {
                thread::spawn(move || {
                    let mut reader = sock.try_clone().unwrap();
                    let mut writer = sock;
                    let _ = io::copy(&mut reader, &mut writer);
                });
            }
        });
        port
    }

    /// A drone end served over TLS, both ends a `TlsDuplex`, the window pinned to the key: a
    /// stream opens while the window's reader already waits on the idle socket — which a session
    /// locked across reads never lets the writer past — and carries bytes both ways.
    #[test]
    fn a_mux_over_tls_opens_while_its_reader_waits() {
        let cert = CertificateDer::from_pem_slice(CERT).unwrap();
        let key = PrivateKeyDer::from_pem_slice(KEY).unwrap();
        let pin = fingerprint(&cert).unwrap();
        let server_config = Arc::new(
            rustls::ServerConfig::builder_with_provider(crate::tls::provider())
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(vec![cert], key)
                .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut conn = rustls::ServerConnection::new(server_config).unwrap();
            while conn.is_handshaking() {
                conn.complete_io(&mut sock).unwrap();
            }
            let duplex = TlsDuplex::new(conn, sock);
            let mux = Mux::drone(duplex.clone(), duplex, None).unwrap();
            crate::serve(&mux, &AllowList::any());
        });

        let (config, _) = client_config(Trust::Pinned(pin));
        let mut conn =
            rustls::ClientConnection::new(config, ServerName::try_from("127.0.0.1").unwrap())
                .unwrap();
        let mut sock = TcpStream::connect(addr).unwrap();
        while conn.is_handshaking() {
            conn.complete_io(&mut sock).unwrap();
        }
        let duplex = TlsDuplex::new(conn, sock);
        let mux = Mux::window(duplex.clone(), duplex, None, None).unwrap();
        thread::sleep(Duration::from_millis(100));

        let port = echo_server();
        for _ in 0..2 {
            let stream = mux.open("127.0.0.1", port).expect("the open is answered");
            let payload = vec![7u8; 100_000];
            stream.send(&payload).unwrap();
            let mut got = 0usize;
            while got < payload.len() {
                match stream.recv() {
                    Recv::Data(data) => {
                        stream.ack(data.len());
                        got += data.len();
                    }
                    other => panic!("the stream ended early: {other:?}"),
                }
            }
            stream.reset(None);
        }
        mux.close();
    }
}
