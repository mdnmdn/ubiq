//! The first frames on a carrier, and the frames that keep it alive.
//!
//! Driven against `ubiq_drone::carrier::serve` over a real loopback pair rather than against the
//! handshake helpers alone, because what these tests are about is *ordering*: that the hello comes
//! before anything else, that a refusal happens before a relay exists, and that a ping is answered
//! by the pump without the relay hearing about it. None of that is visible from the helpers, whose
//! own round trips are unit-tested in `ubiq_proto::carrier`.
//!
//! Every assertion here reads raw frames off the socket instead of attaching a `bus::Client`, for
//! the same reason: a client would hide exactly the thing under test — which frames crossed the
//! wire, in which order, and which ones never did.

use std::io::Write;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::time::Duration;

use ubiq_drone::relay::Relay;
use ubiq_host::carrier::NoCloser;
use ubiq_proto::carrier::{HandshakeError, welcome};
use ubiq_proto::messages::Message;
use ubiq_proto::wire::{self, MESSAGE_SCHEMA, WireError};

/// How long a read waits before the test calls the drone hung. Generous, because a cold process on
/// a loaded machine is slow and a flaky test is worse than a slow one.
const PATIENCE: Duration = Duration::from_secs(20);

/// The Ubiq end of a carrier a drone is serving: the socket, and the thread the drone runs on.
struct FarEnd {
    stream: TcpStream,
    drone: std::thread::JoinHandle<Result<(), HandshakeError>>,
}

impl FarEnd {
    /// Start a drone on one end of a loopback pair and hand back the other end. Nothing has been
    /// exchanged yet: the caller is the one that decides how the handshake goes.
    fn open(roots: Vec<std::path::PathBuf>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let address = listener.local_addr().expect("the bound address");
        let stream = TcpStream::connect(address).expect("dialling the drone");
        let (near, _) = listener.accept().expect("accepting the drone");
        stream
            .set_read_timeout(Some(PATIENCE))
            .expect("a read timeout");

        let drone = std::thread::spawn(move || {
            let reader = near.try_clone().expect("the drone's read half");
            ubiq_drone::carrier::serve(
                Box::new(reader),
                Box::new(near),
                Box::new(NoCloser),
                Relay::new(roots),
            )
        });

        Self { stream, drone }
    }

    fn say(&mut self, message: Message) {
        wire::write_frame(&mut self.stream, &message).expect("writing a frame");
        self.stream.flush().expect("flushing");
    }

    fn next_frame(&mut self) -> Message {
        wire::read_frame(&mut self.stream).expect("a frame from the drone")
    }

    /// End the session and give back what the drone's `serve` returned.
    fn finish(self) -> Result<(), HandshakeError> {
        let _ = self.stream.shutdown(Shutdown::Both);
        self.drone.join().expect("the drone thread")
    }
}

/// A drone whose schema matches is accepted, and the session behind the handshake is an ordinary
/// one: it greets, and it answers.
#[test]
fn a_matching_schema_completes_the_handshake_and_serves() {
    let project = tempfile::tempdir().expect("a project folder");
    let mut far = FarEnd::open(vec![project.path().to_path_buf()]);

    let mut reader = far.stream.try_clone().expect("the read half");
    let mut writer = far.stream.try_clone().expect("the write half");
    let identity = welcome(&mut reader, &mut writer, "0.1.0").expect("an accepted drone");

    assert_eq!(identity.message_schema, MESSAGE_SCHEMA);
    assert_eq!(identity.os, std::env::consts::OS);
    assert!(identity.has("files"), "a relay drone serves files");
    assert!(
        !identity.has("harness") && !identity.has("search") && !identity.has("git"),
        "a relay drone advertises nothing it would only refuse: {:?}",
        identity.capabilities
    );

    // The relay exists only because the handshake said so, and it greets on attach exactly as it
    // does without one.
    assert!(
        matches!(far.next_frame(), Message::HostInfo { .. }),
        "an accepted drone greets its client"
    );

    far.say(Message::ListProjects);
    match far.next_frame() {
        Message::ProjectList { projects } => assert_eq!(projects.len(), 1, "one root, one project"),
        other => panic!("expected ProjectList, got {other:?}"),
    }

    far.finish().expect("a session that served");
}

/// A drone whose schema does not match is turned away with a sentence, and never gets as far as
/// having a relay — so there is nothing for a pane to be spawned by.
#[test]
fn a_mismatched_schema_is_refused_and_nothing_is_ever_spawned() {
    let project = tempfile::tempdir().expect("a project folder");
    let mut far = FarEnd::open(vec![project.path().to_path_buf()]);

    // The hello is the drone's first frame, before anything else it would ever write.
    let schema = match far.next_frame() {
        Message::DroneHello { message_schema, .. } => message_schema,
        other => panic!("a drone's first frame is its hello, not {other:?}"),
    };
    assert_eq!(schema, MESSAGE_SCHEMA);

    // Stand in for an Ubiq built at a different revision. `welcome` cannot be used here: it
    // compares against this build's own schema, which by construction matches.
    let reason = "This Ubiq speaks message schema 99; the drone speaks 1.";
    far.say(Message::DroneReady {
        ubiq_version: "9.9.9".to_string(),
        message_schema: MESSAGE_SCHEMA + 98,
        accepted: false,
        reason: Some(reason.to_string()),
    });

    // Nothing follows. A relay that had been started would have greeted with `HostInfo` the moment
    // the pump attached its client — so an end of stream here is the proof that no relay, no
    // client and therefore no pseudo-terminal ever existed.
    match wire::read_frame(&mut far.stream) {
        Err(WireError::Eof) => {}
        Ok(frame) => panic!("a refused drone served something: {frame:?}"),
        Err(other) => panic!("expected a clean close, got {other}"),
    }

    let refusal = far.finish().expect_err("a refused handshake");
    match refusal {
        HandshakeError::Refused(said) => assert_eq!(said, reason, "the reason is carried verbatim"),
        other => panic!("expected a refusal, got {other}"),
    }
}

/// A ping is the pump's business and nobody else's: it is answered on the spot, the answer is
/// swallowed at the other end, and the relay behind it never hears either.
#[test]
fn a_ping_is_answered_by_the_pump_and_never_reaches_the_relay() {
    let project = tempfile::tempdir().expect("a project folder");
    let mut far = FarEnd::open(vec![project.path().to_path_buf()]);

    let mut reader = far.stream.try_clone().expect("the read half");
    let mut writer = far.stream.try_clone().expect("the write half");
    welcome(&mut reader, &mut writer, "0.1.0").expect("an accepted drone");
    assert!(matches!(far.next_frame(), Message::HostInfo { .. }));

    far.say(Message::Ping { nonce: 4242 });
    match far.next_frame() {
        Message::Pong { nonce } => assert_eq!(nonce, 4242, "the nonce is echoed, not invented"),
        other => panic!("expected a Pong, got {other:?}"),
    }

    // A `Pong` arriving unasked is swallowed the same way, and then an ordinary request is
    // answered next — with nothing between it and the answer. Had either frame reached the
    // relay's dispatch, its catch-all would have refused or logged it and there would be no Pong
    // at all: the pump is the only thing in this process that knows what a heartbeat is.
    far.say(Message::Pong { nonce: 7 });
    far.say(Message::ListProjects);
    match far.next_frame() {
        Message::ProjectList { projects } => assert_eq!(projects.len(), 1),
        other => panic!("a heartbeat disturbed the session: {other:?}"),
    }

    far.finish().expect("a session that served");
}
