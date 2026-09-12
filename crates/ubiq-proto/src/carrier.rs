//! What both ends of a byte stream must agree on: how a drone introduces itself, and how either
//! end notices the other has gone quiet.
//!
//! **Why this is here and not in a pump.** There are two pumps and there always will be:
//! `ubiq_host::carrier::pump` on the host side, and `spawn_pump` in
//! `crates/ubiq/src/app/remote_connect.rs` on the interface's, because `crates/ubiq` does not
//! depend on `crates/ubiq-host` and `just ui` enforces it. The thread shape is duplicated on
//! purpose; the *rules* must not be, or the two halves drift into disagreeing about how long
//! silence is allowed to last. Everything a pump has to get right and could get differently lives
//! in this module, and both pumps call it.
//!
//! Nothing here reads or writes a socket of its own beyond the handshake's four frames: a
//! [`Heartbeat`] is a small state machine the pump drives, so the pump keeps owning its threads
//! and its stream.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use crate::messages::Message;
use crate::wire::{self, MESSAGE_SCHEMA, WireError};

// ── The heartbeat ───────────────────────────────────────────────────────────

/// How long a carrier may be silent before one end asks whether the other is still there.
///
/// **Tens of seconds, not seconds.** The thing being detected is a connection that died without
/// saying so — a suspended laptop, an expired NAT entry, an `ssh` whose far side was killed — and
/// none of those are urgent to the millisecond: a pane the user is typing into reports a dead
/// stream on the next write regardless, and an idle one has nobody waiting. What a short interval
/// would buy is a faster answer to a question nobody asked; what it costs is a wakeup, a frame and
/// a log-worthy round trip on every idle session on every machine, forever. Twenty seconds is
/// comfortably inside the shortest NAT idle timeouts that are common in practice (typically 30 to
/// 120 seconds for TCP), which also makes it a keepalive and not only a detector.
pub const PING_INTERVAL: Duration = Duration::from_secs(20);

/// How many pings may go unanswered before the peer is treated as gone.
///
/// Three, which with [`PING_INTERVAL`] means roughly a minute of total silence. One would make a
/// single dropped packet or a stalled process look like a death; a dozen would make a dead session
/// linger past the point the user has already noticed. A minute is long enough that a machine
/// paging back in survives it, and short enough that phase 6's lingering drone starts its
/// countdown from something close to when its client actually left.
pub const MISSED_PINGS_BEFORE_GONE: u32 = 3;

/// What a pump should do with one frame it has just read.
#[derive(Debug)]
pub enum Beat {
    /// An ordinary message. Hand it to the hub, or to the interface's delivery channel.
    Deliver(Message),
    /// A [`Message::Ping`]. Write this answer back and deliver nothing.
    Answer(Message),
    /// A [`Message::Pong`]. The peer is alive, and that is the whole of its content — it reaches
    /// neither the coordinator nor `AppState`.
    Swallow,
}

/// Whether a carrier's peer is still answering, and when to ask.
///
/// Driven by both threads of a pump: the reader calls [`Heartbeat::inbound`] on every frame, and
/// the writer calls [`Heartbeat::due`] and [`Heartbeat::is_gone`] whenever it has nothing else to
/// do. It is therefore shared behind a lock, and every method is short enough that holding one is
/// free.
///
/// **It is quiet while frames are flowing.** A ping is due only when nothing at all has arrived
/// for [`PING_INTERVAL`], so a session with a live pane in it — where a dead stream is discovered
/// by the next failed write anyway — never puts a single extra frame on the wire.
pub struct Heartbeat {
    /// When a frame of any kind last arrived. The evidence a ping exists to ask for, which is why
    /// ordinary traffic counts as an answer.
    last_inbound: Instant,
    /// When this end last sent a ping, so the misses are counted one interval apart rather than
    /// as fast as the writer happens to wake.
    last_ping: Instant,
    /// Pings sent since the last frame arrived.
    missed: u32,
    /// The next nonce. Monotonic rather than random: it is an echo check inside a stream that is
    /// already ordered and already authenticated by whatever opened it, not a secret.
    next_nonce: u64,
}

impl Default for Heartbeat {
    fn default() -> Self {
        Self::new()
    }
}

impl Heartbeat {
    /// A heartbeat on a carrier that has just been established. The clock starts now, so the first
    /// ping on a session nobody uses is one [`PING_INTERVAL`] away.
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            last_inbound: now,
            last_ping: now,
            missed: 0,
            next_nonce: 1,
        }
    }

    /// Classify one frame the reader has just decoded, and record that the peer is alive.
    ///
    /// Takes the message by value and hands it back in [`Beat::Deliver`] so a pump cannot deliver
    /// a `Ping` by forgetting to match on it.
    pub fn inbound(&mut self, message: Message) -> Beat {
        self.last_inbound = Instant::now();
        self.missed = 0;
        match message {
            Message::Ping { nonce } => Beat::Answer(Message::Pong { nonce }),
            Message::Pong { .. } => Beat::Swallow,
            other => Beat::Deliver(other),
        }
    }

    /// The ping to send now, if one is owed.
    ///
    /// `None` while frames are arriving, and `None` again until a whole [`PING_INTERVAL`] after
    /// the last one — so a writer that wakes five times a second still sends at most one ping per
    /// interval. Counts the ping as missed the moment it is sent; the answer clears it.
    pub fn due(&mut self) -> Option<Message> {
        let now = Instant::now();
        if now.duration_since(self.last_inbound) < PING_INTERVAL {
            return None;
        }
        if now.duration_since(self.last_ping) < PING_INTERVAL {
            return None;
        }
        self.last_ping = now;
        self.missed += 1;
        let nonce = self.next_nonce;
        self.next_nonce = self.next_nonce.wrapping_add(1);
        Some(Message::Ping { nonce })
    }

    /// Whether the peer has now missed enough pings to be treated as gone.
    ///
    /// A pump that sees this tears the session down exactly as it does for a dropped socket —
    /// closing its half and letting the other direction end — rather than inventing a second
    /// shutdown path. A second path is a second chance to leave a pane running.
    pub fn is_gone(&self) -> bool {
        self.missed >= MISSED_PINGS_BEFORE_GONE
    }
}

// ── The handshake ───────────────────────────────────────────────────────────

/// What a drone said about itself, once its hello has been accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DroneIdentity {
    pub drone_version: String,
    pub os: String,
    pub arch: String,
    pub triplet: String,
    pub message_schema: u32,
    pub capabilities: Vec<String>,
}

impl DroneIdentity {
    /// Whether the drone advertised a named capability. A string set rather than a bitfield, so a
    /// build that gains one is understood by a peer that has never heard the name.
    pub fn has(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|name| name == capability)
    }
}

/// Why a handshake did not produce a session.
#[derive(Debug)]
pub enum HandshakeError {
    /// The stream failed, or ended, before the exchange finished.
    Wire(WireError),
    /// A frame arrived where the handshake's own was expected. The carrier is unusable: the first
    /// frames on a drone stream are fixed, and anything else means the far end is not a drone, or
    /// not an Ubiq, or is mid-session from a previous life.
    Unexpected(String),
    /// The far end declined, and this is the sentence it gave. Shown as-is; it is written to be
    /// readable in a modal.
    Refused(String),
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandshakeError::Wire(err) => {
                write!(f, "the carrier failed during the handshake: {err}")
            }
            HandshakeError::Unexpected(what) => {
                write!(f, "expected a handshake frame, got {what}")
            }
            HandshakeError::Refused(reason) => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for HandshakeError {}

impl From<WireError> for HandshakeError {
    fn from(err: WireError) -> Self {
        HandshakeError::Wire(err)
    }
}

/// The hello a drone introduces itself with, filled in from the build it is running.
///
/// The machine facts are the compile-time constants `ubiq_drone::probe_line` uses, for the same
/// reason: this runs before anything is served, and must not depend on a sampler thread or a
/// scratch directory existing.
pub fn hello(drone_version: &str, capabilities: &[&str]) -> Message {
    Message::DroneHello {
        drone_version: drone_version.to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        triplet: format!(
            "{}-{}-{}",
            std::env::consts::ARCH,
            std::env::consts::OS,
            std::env::consts::FAMILY
        ),
        message_schema: MESSAGE_SCHEMA,
        capabilities: capabilities.iter().map(|name| name.to_string()).collect(),
    }
}

/// The drone's half of the handshake: say hello, and wait to be told to serve.
///
/// Returns once a [`Message::DroneReady`] with `accepted` has arrived, and the caller may then
/// start its relay and its pump on the same two halves. Anything else — a refusal, a frame that is
/// not the answer, a stream that ends — is an error, and the caller spawns nothing.
///
/// **It flushes.** A drone's write half is standard output, which the standard library buffers by
/// line; a length-prefixed binary frame carries no newline, so an unflushed hello would sit in the
/// buffer while both ends waited for the other.
pub fn greet<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    hello: &Message,
) -> Result<(), HandshakeError> {
    wire::write_frame(writer, hello)?;
    writer.flush().map_err(WireError::Io)?;

    match wire::read_frame(reader)? {
        Message::DroneReady {
            accepted: true,
            ubiq_version,
            message_schema,
            ..
        } => {
            tracing::info!("ubiq {ubiq_version} accepted the carrier at schema {message_schema}");
            Ok(())
        }
        Message::DroneReady {
            accepted: false,
            reason,
            ..
        } => Err(HandshakeError::Refused(reason.unwrap_or_else(|| {
            "Ubiq refused the connection without saying why.".to_string()
        }))),
        other => Err(HandshakeError::Unexpected(format!("{other:?}"))),
    }
}

/// Ubiq's half of the handshake: read the drone's hello, decide, and answer.
///
/// The decision is the schema comparison and nothing else — capabilities are advertised, never
/// demanded, so a drone that offers less than this build hoped for still connects and is refused
/// per message by its own relay. A mismatch is answered with `accepted: false` and a sentence,
/// then reported here as [`HandshakeError::Refused`] so the caller closes its end too: both sides
/// close cleanly, and neither is left waiting on a session that will not happen.
pub fn welcome<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    ubiq_version: &str,
) -> Result<DroneIdentity, HandshakeError> {
    let identity = match wire::read_frame(reader)? {
        Message::DroneHello {
            drone_version,
            os,
            arch,
            triplet,
            message_schema,
            capabilities,
        } => DroneIdentity {
            drone_version,
            os,
            arch,
            triplet,
            message_schema,
            capabilities,
        },
        other => return Err(HandshakeError::Unexpected(format!("{other:?}"))),
    };

    let mismatch = (identity.message_schema != MESSAGE_SCHEMA).then(|| schema_refusal(&identity));
    let answer = Message::DroneReady {
        ubiq_version: ubiq_version.to_string(),
        message_schema: MESSAGE_SCHEMA,
        accepted: mismatch.is_none(),
        reason: mismatch.clone(),
    };
    wire::write_frame(writer, &answer)?;
    writer.flush().map_err(WireError::Io)?;

    match mismatch {
        Some(reason) => Err(HandshakeError::Refused(reason)),
        None => Ok(identity),
    }
}

/// The sentence a schema mismatch is refused with. A modal shows it verbatim, so it names both
/// numbers, both versions, and the thing the user can actually do about it.
fn schema_refusal(identity: &DroneIdentity) -> String {
    format!(
        "This Ubiq speaks message schema {MESSAGE_SCHEMA}; the drone on the other end \
         (ubiq-drone {}, {}) speaks {}. Deploy a drone built from the same revision.",
        identity.drone_version, identity.triplet, identity.message_schema
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_matching_schema_is_accepted_and_the_identity_survives() {
        let mut drone_out: Vec<u8> = Vec::new();
        wire::write_frame(&mut drone_out, &hello("0.1.0", &["files"])).expect("the hello");

        let mut ubiq_in = std::io::Cursor::new(drone_out);
        let mut ubiq_out: Vec<u8> = Vec::new();
        let identity = welcome(&mut ubiq_in, &mut ubiq_out, "0.1.0").expect("an accepted hello");

        assert_eq!(identity.message_schema, MESSAGE_SCHEMA);
        assert!(identity.has("files"));
        assert!(!identity.has("harness"));

        let mut back = std::io::Cursor::new(ubiq_out);
        match wire::read_frame(&mut back).expect("the answer") {
            Message::DroneReady {
                accepted, reason, ..
            } => {
                assert!(accepted);
                assert!(reason.is_none());
            }
            other => panic!("expected DroneReady, got {other:?}"),
        }
    }

    #[test]
    fn a_mismatched_schema_is_refused_with_a_readable_reason() {
        let mut drone_out: Vec<u8> = Vec::new();
        let hello = Message::DroneHello {
            drone_version: "9.9.9".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            triplet: "x86_64-linux-unix".to_string(),
            message_schema: MESSAGE_SCHEMA + 7,
            capabilities: vec!["files".to_string()],
        };
        wire::write_frame(&mut drone_out, &hello).expect("the hello");

        let mut ubiq_in = std::io::Cursor::new(drone_out);
        let mut ubiq_out: Vec<u8> = Vec::new();
        let refusal = welcome(&mut ubiq_in, &mut ubiq_out, "0.1.0").expect_err("a refusal");
        let HandshakeError::Refused(reason) = refusal else {
            panic!("expected a refusal");
        };
        assert!(reason.contains("9.9.9"), "the reason names the drone build");

        // The drone reading that same answer refuses too, with the same sentence.
        let mut drone_in = std::io::Cursor::new(ubiq_out);
        let mut sink: Vec<u8> = Vec::new();
        let err = greet(&mut drone_in, &mut sink, &hello).expect_err("a refused greet");
        assert!(matches!(err, HandshakeError::Refused(said) if said == reason));
    }

    #[test]
    fn a_ping_is_answered_and_a_pong_is_swallowed() {
        let mut beat = Heartbeat::new();
        match beat.inbound(Message::Ping { nonce: 42 }) {
            Beat::Answer(Message::Pong { nonce }) => assert_eq!(nonce, 42),
            other => panic!("expected a Pong, got {other:?}"),
        }
        assert!(matches!(
            beat.inbound(Message::Pong { nonce: 42 }),
            Beat::Swallow
        ));
        assert!(matches!(
            beat.inbound(Message::ListShells),
            Beat::Deliver(Message::ListShells)
        ));
    }

    #[test]
    fn no_ping_is_due_while_frames_are_arriving() {
        let mut beat = Heartbeat::new();
        assert!(beat.due().is_none());
        assert!(!beat.is_gone());
    }

    #[test]
    fn silence_produces_one_ping_per_interval_and_then_a_death() {
        let mut beat = Heartbeat::new();
        // Age the clock rather than sleeping a minute: the fields are the whole state.
        let long_ago = Instant::now() - PING_INTERVAL * 10;
        beat.last_inbound = long_ago;
        beat.last_ping = long_ago;

        for _ in 0..MISSED_PINGS_BEFORE_GONE {
            assert!(beat.due().is_some(), "a ping is owed after the interval");
            // A second wake inside the same interval sends nothing.
            assert!(beat.due().is_none(), "one ping per interval, not per wake");
            beat.last_ping -= PING_INTERVAL;
        }
        assert!(beat.is_gone());

        // One frame of any kind clears the whole count.
        beat.inbound(Message::ListShells);
        assert!(!beat.is_gone());
    }
}
