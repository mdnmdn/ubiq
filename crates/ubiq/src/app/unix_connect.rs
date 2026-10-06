//! Reach a drone over a local Unix socket: connect the path, shake hands on it, and hand the two
//! halves to the same pump an `ssh` dial uses.
//!
//! The drone is one already listening — `ubiq-drone --listen`, or anything else serving the drone
//! carrier on a socket path on this machine. Nothing is spawned and nothing is deployed: the path
//! either answers with a drone hello or the dial fails with the reason.
//!
//! **Never on the GPUI thread.** The connect and the handshake read block;
//! `AppState::try_connect_remote` and the reconnect loop run this inside `cx.background_spawn`.

use std::io::{Read, Write};
#[cfg(unix)]
use std::time::Duration;

use ubiq_proto::bus::Client;
use ubiq_proto::carrier::{self, DroneIdentity, HandshakeError};

use super::remote_connect::{ConnectFailure, start_drone_session};

/// How long the far end may take to say hello. A socket file whose owner is not a drone (or is a
/// wedged one) would otherwise hang the modal on a read nothing answers.
#[cfg(unix)]
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// What a successful dial hands back: the `Client` the bus registers and what the drone said
/// about itself.
pub struct UnixDial {
    pub client: Client,
    pub identity: DroneIdentity,
}

/// Dial a drone listening on a Unix socket at `path`. Blocking.
#[cfg(unix)]
pub fn dial_unix(path: &str) -> Result<UnixDial, ConnectFailure> {
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;

    let stream = UnixStream::connect(path)
        .map_err(|error| ConnectFailure::Unreachable(format!("{path}: {error}")))?;
    let io = |error: std::io::Error| ConnectFailure::Io(error.to_string());
    let reader = stream.try_clone().map_err(io)?;
    let closer = stream.try_clone().map_err(io)?;
    // The read timeout is on the socket, so it covers the cloned reader too; cleared once the
    // handshake lands so a session that runs for hours is never mistaken for a stalled read.
    stream
        .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(io)?;
    let outcome = handshake_and_pump(
        reader,
        stream.try_clone().map_err(io)?,
        Box::new(move || {
            let _ = closer.shutdown(Shutdown::Both);
        }),
        || stream.set_read_timeout(None),
    );
    match outcome {
        Ok((client, identity)) => Ok(UnixDial { client, identity }),
        Err(error) => Err(match error {
            HandshakeError::Refused(reason) => ConnectFailure::Refused(reason),
            HandshakeError::Wire(wire) => ConnectFailure::Io(wire.to_string()),
            other => ConnectFailure::Refused(format!("not a drone at {path}: {other}")),
        }),
    }
}

/// On a platform with no Unix sockets the variant still exists — a settings file travels — but
/// there is nothing to dial it with.
#[cfg(not(unix))]
pub fn dial_unix(path: &str) -> Result<UnixDial, ConnectFailure> {
    Err(ConnectFailure::Unreachable(format!(
        "local sockets are not supported on this platform ({path})"
    )))
}

/// The handshake and the pump over any reader/writer pair: [`carrier::welcome`], then — only on
/// success — [`start_drone_session`]. `after_welcome` runs between the two, for a carrier that
/// has a deadline to lift. On failure the halves are dropped and no session starts.
fn handshake_and_pump<R, W>(
    mut reader: R,
    mut writer: W,
    closer: Box<dyn FnOnce() + Send>,
    after_welcome: impl FnOnce() -> std::io::Result<()>,
) -> Result<(Client, DroneIdentity), HandshakeError>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    let identity = carrier::welcome(&mut reader, &mut writer, env!("CARGO_PKG_VERSION"))?;
    after_welcome()
        .map_err(|error| HandshakeError::Wire(ubiq_proto::wire::WireError::Io(error)))?;
    Ok((start_drone_session(reader, writer, closer), identity))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::thread;
    use ubiq_proto::wire::MESSAGE_SCHEMA;

    /// The drone's half, served by the library's own `greet` on the far end of a socket pair.
    fn drone_side(
        mut far: UnixStream,
        hello: ubiq_proto::messages::Message,
    ) -> thread::JoinHandle<bool> {
        thread::spawn(move || {
            let mut read = far.try_clone().expect("a clone");
            carrier::greet(&mut read, &mut far, &hello).is_ok()
        })
    }

    #[test]
    fn a_drone_on_a_socket_pair_completes_the_handshake() {
        let (near, far) = UnixStream::pair().expect("a pair");
        let drone = drone_side(far, carrier::hello("9.9.9", &["files"]));
        let reader = near.try_clone().expect("a clone");
        let (_client, identity) =
            handshake_and_pump(reader, near, Box::new(|| {}), || Ok(())).expect("a handshake");
        assert_eq!(identity.drone_version, "9.9.9");
        assert_eq!(identity.message_schema, MESSAGE_SCHEMA);
        assert!(identity.has("files"));
        assert!(drone.join().expect("the drone thread"));
    }

    #[test]
    fn a_peer_that_is_not_a_drone_fails_the_handshake() {
        let (near, far) = UnixStream::pair().expect("a pair");
        drop(far);
        let reader = near.try_clone().expect("a clone");
        let refused = handshake_and_pump(reader, near, Box::new(|| {}), || Ok(()));
        assert!(refused.is_err());
    }

    #[test]
    fn dialing_a_path_nobody_listens_on_is_unreachable() {
        let failure = dial_unix("/nonexistent/ubiq-drone.sock")
            .err()
            .expect("a failure");
        assert!(matches!(failure, ConnectFailure::Unreachable(_)));
    }

    #[test]
    fn dialing_a_listening_drone_attaches() {
        let dir = std::env::temp_dir().join(format!("ubiq-unix-dial-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a dir");
        let path = dir.join("drone.sock");
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).expect("a listener");
        let drone = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("a connection");
            let mut read = stream.try_clone().expect("a clone");
            carrier::greet(&mut read, &mut stream, &carrier::hello("1.2.3", &[])).is_ok()
        });
        let dial = dial_unix(path.to_str().expect("utf-8")).expect("a dial");
        assert_eq!(dial.identity.drone_version, "1.2.3");
        assert!(drone.join().expect("the drone thread"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
