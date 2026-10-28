//! The tunnel channel: TCP streams between a window and a drone, **off the bus** (`D227`).
//!
//! A tunnel never rides the [`crate::messages::Message`] stream. Each carrier opens a second,
//! dedicated channel beside the one the bus runs on — over RDP a second dynamic virtual channel,
//! [`TUNNEL_CHANNEL`], which the `--rdp` child bridges to a Unix socket of its own; over `ssh` a
//! second exec — and speaks [`TunnelFrame`] on it, length-prefixed MessagePack exactly as
//! [`crate::wire`] frames a message ([`write_frame`], [`read_frame`]). A drone that can serve the
//! channel advertises [`crate::carrier::CAP_TUNNEL`].
//!
//! The window accepts a local socket (a [`crate::settings::TunnelSpec`]'s `local_port`), mints a
//! [`crate::ids::TunnelStreamId`] and sends [`TunnelFrame::Open`]; the drone dials `host:port` on
//! its side and answers `Opened` or `Refused`. Bytes then flow both ways as `Data`, each direction
//! credit-limited: a receiver returns `Ack` once it has written the bytes to its socket, and a
//! sender never has more than the stream's window unacknowledged. There is no channel-wide cap:
//! one would let streams whose consumer stopped reading starve every other stream.

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use crate::ids::TunnelStreamId;
use crate::wire::{self, WireError};

/// The RDP dynamic virtual channel the tunnel runs on, beside the bus's own.
pub const TUNNEL_CHANNEL: &str = "ubiq::tunnel::v1";

/// The version a [`TunnelFrame::Hello`] states. Bumped by hand on a breaking frame change.
pub const TUNNEL_VERSION: u32 = 1;

/// The largest `Data.bytes` either end sends. A larger read is split.
pub const TUNNEL_CHUNK_MAX: usize = 16 * 1024;

/// The default per-stream credit window, the `window` an `Open` offers.
pub const TUNNEL_STREAM_WINDOW: u32 = 64 * 1024;

/// Which half of a stream a [`TunnelFrame::Close`] ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelHalf {
    /// The sender will write no more; it still reads — TCP's half-close, which an HTTP client
    /// sending its request and waiting for the answer depends on.
    Write,
    /// The stream is gone both ways.
    Both,
}

/// One frame on the tunnel channel. Every frame but `Hello` names its stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TunnelFrame {
    /// The first frame each end writes. `rate_bps` is the window's ceiling on the channel's
    /// traffic ([`crate::settings::SavedRemoteHost::tunnel_rate_bps`]); `None` is unlimited.
    Hello { version: u32, rate_bps: Option<u64> },
    /// Window → drone: dial `host:port` for a socket I accepted. `window` is the credit the drone
    /// may send before an ack.
    Open {
        stream: TunnelStreamId,
        host: String,
        port: u16,
        window: u32,
    },
    /// Drone → window: the dial succeeded; data may flow.
    Opened { stream: TunnelStreamId },
    /// Drone → window: the dial failed, with a sentence. The stream is over.
    Refused { stream: TunnelStreamId, error: String },
    /// Either way: bytes read from one socket, for the other. At most [`TUNNEL_CHUNK_MAX`].
    Data {
        stream: TunnelStreamId,
        #[serde(with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    /// Either way: `bytes` of the peer's data have been written to the socket — credit returned.
    Ack { stream: TunnelStreamId, bytes: u32 },
    /// Either way: one half, or the whole stream, is done. `error` set is an abnormal end.
    Close {
        stream: TunnelStreamId,
        half: TunnelHalf,
        error: Option<String>,
    },
}

/// Write one frame, [`crate::wire::write_frame`]'s framing.
pub fn write_frame<W: Write>(w: &mut W, frame: &TunnelFrame) -> Result<(), WireError> {
    wire::write_value_frame(w, frame)
}

/// Read one frame, [`crate::wire::read_frame`]'s framing — the same ceiling, the same clean EOF.
pub fn read_frame<R: Read>(r: &mut R) -> Result<TunnelFrame, WireError> {
    wire::read_value_frame(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_data_stays_binary() {
        let stream = TunnelStreamId::generate();
        let frames = [
            TunnelFrame::Hello {
                version: TUNNEL_VERSION,
                rate_bps: Some(1_000_000),
            },
            TunnelFrame::Open {
                stream,
                host: "localhost".into(),
                port: 5432,
                window: TUNNEL_STREAM_WINDOW,
            },
            TunnelFrame::Data {
                stream,
                bytes: vec![0x41; 10_000],
            },
            TunnelFrame::Close {
                stream,
                half: TunnelHalf::Write,
                error: None,
            },
        ];
        let mut buf = Vec::new();
        for frame in &frames {
            write_frame(&mut buf, frame).unwrap();
        }
        // `serde_bytes`: a 10 000-byte chunk costs about 10 000 bytes, not one int per byte.
        assert!(buf.len() < 12_000, "{} bytes", buf.len());
        let mut cursor = std::io::Cursor::new(buf);
        for frame in &frames {
            assert_eq!(&read_frame(&mut cursor).unwrap(), frame);
        }
        assert!(matches!(read_frame(&mut cursor), Err(WireError::Eof)));
    }
}
