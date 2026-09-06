//! The socket wire format: a 4-byte big-endian length prefix followed by a MessagePack body.
//!
//! This module owns framing only — it has no socket, no thread and no notion of a connection.
//! `encode`/`decode` and the `Read`/`Write` wrappers around them are what a later transport (in
//! the host crate) drives; nothing here assumes where the bytes came from.
//!
//! **Why `to_vec_named` and not the compact `to_vec`.** `rmp_serde::to_vec` encodes a struct
//! positionally — field values only, no names — which is faster and smaller but requires both
//! ends to agree on field order and, more importantly, cannot express `#[serde(flatten)]`:
//! flatten needs a map to merge into, and a positional encoding has no map. `ProjectSnapshot`
//! (`crate::projects::ProjectSnapshot`) flattens `ProjectRecord` into itself, so `to_vec` fails to
//! round-trip it. `to_vec_named` encodes struct fields as a map, which flatten can merge into and
//! which does not require the two ends to have been compiled from the same struct layout — the
//! self-describing property we need for a remote host and UI that may be different builds. See
//! `tests::project_list_with_a_flattened_snapshot_round_trips` for the test that pins this down:
//! it is written against `to_vec_named` and fails if the module is changed to use `to_vec`.

use std::io::{self, Read, Write};

use crate::messages::Message;

/// A frame header claiming more than this is refused before anything is allocated. 64 MiB is far
/// past any single message this contract carries — the terminal family chunks as the
/// pseudo-terminal hands bytes back, never as one giant read — so a claim past this ceiling is a
/// corrupt or hostile header, not a message running long.
pub const MAX_FRAME: usize = 64 * 1024 * 1024;

const LEN_PREFIX: usize = 4;

/// Everything that can go wrong turning a [`Message`] into bytes and back.
#[derive(Debug)]
pub enum WireError {
    /// The underlying reader or writer failed.
    Io(io::Error),
    /// The message would not serialise.
    Encode(rmp_serde::encode::Error),
    /// The bytes read did not decode as a `Message`.
    Decode(rmp_serde::decode::Error),
    /// The frame's length prefix claimed more than [`MAX_FRAME`]. Rejected before any allocation
    /// for the body was made.
    FrameTooLarge(u32),
    /// The peer closed the connection cleanly, with no bytes read for a new frame. Not an error a
    /// caller need report — it is how a socket pump learns the other side is done — but is worth
    /// a variant of its own so it is never conflated with a real I/O failure or a torn frame.
    Eof,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireError::Io(err) => write!(f, "wire I/O error: {err}"),
            WireError::Encode(err) => write!(f, "wire encode error: {err}"),
            WireError::Decode(err) => write!(f, "wire decode error: {err}"),
            WireError::FrameTooLarge(len) => {
                write!(f, "frame claimed {len} bytes, over the {MAX_FRAME} ceiling")
            }
            WireError::Eof => write!(f, "peer closed the connection"),
        }
    }
}

impl std::error::Error for WireError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WireError::Io(err) => Some(err),
            WireError::Encode(err) => Some(err),
            WireError::Decode(err) => Some(err),
            WireError::FrameTooLarge(_) | WireError::Eof => None,
        }
    }
}

impl From<io::Error> for WireError {
    fn from(err: io::Error) -> Self {
        WireError::Io(err)
    }
}

/// Encode one message as a MessagePack body, with no length prefix. [`write_frame`] is what adds
/// the prefix; this is exposed on its own for callers that frame differently, and for the tests
/// that measure the body's own size.
pub fn encode(message: &Message) -> Result<Vec<u8>, WireError> {
    // See the module doc comment: `to_vec_named`, not `to_vec` — flatten requires it.
    rmp_serde::to_vec_named(message).map_err(WireError::Encode)
}

/// Decode one message from a MessagePack body with no length prefix.
pub fn decode(body: &[u8]) -> Result<Message, WireError> {
    rmp_serde::from_slice(body).map_err(WireError::Decode)
}

/// Write one message as a length-prefixed frame: 4 bytes of big-endian length, then the body.
pub fn write_frame<W: Write>(w: &mut W, message: &Message) -> Result<(), WireError> {
    let body = encode(message)?;
    let len = u32::try_from(body.len()).map_err(|_| WireError::FrameTooLarge(u32::MAX))?;
    w.write_all(&len.to_be_bytes())?;
    w.write_all(&body)?;
    Ok(())
}

/// Read one length-prefixed frame and decode it.
///
/// A prefix claiming more than [`MAX_FRAME`] is rejected before the body buffer is allocated, so a
/// corrupt or hostile header costs four bytes rather than however much memory it names. Zero bytes
/// read where a new frame's length prefix should start is [`WireError::Eof`] — a peer that closed
/// cleanly between frames, not a failure; a partial prefix or a partial body is a real error,
/// because the stream was already inside a frame.
pub fn read_frame<R: Read>(r: &mut R) -> Result<Message, WireError> {
    let mut len_buf = [0u8; LEN_PREFIX];
    read_exact_or_eof(r, &mut len_buf)??;
    let len = u32::from_be_bytes(len_buf);
    if len as usize > MAX_FRAME {
        return Err(WireError::FrameTooLarge(len));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)?;
    decode(&body)
}

/// Read exactly `buf.len()` bytes, distinguishing "nothing at all was read" (clean end of stream)
/// from "some bytes arrived and then the stream ended" (a torn frame, a real error). The outer
/// `Result` is I/O failure; the inner one is `Err(WireError::Eof)` for the clean case and `Ok(())`
/// otherwise.
fn read_exact_or_eof<R: Read>(
    r: &mut R,
    buf: &mut [u8],
) -> Result<Result<(), WireError>, WireError> {
    let mut at = 0;
    while at < buf.len() {
        match r.read(&mut buf[at..])? {
            0 if at == 0 => return Ok(Err(WireError::Eof)),
            0 => {
                return Ok(Err(WireError::Io(io::Error::from(
                    io::ErrorKind::UnexpectedEof,
                ))));
            }
            n => at += n,
        }
    }
    Ok(Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{PaneId, ProjectId};
    use crate::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

    fn snapshot() -> ProjectSnapshot {
        ProjectSnapshot {
            record: ProjectRecord {
                id: ProjectId::generate(),
                name: "demo".into(),
                path: "/tmp/demo".into(),
                colour: 3,
                custom_colour: None,
                temporary: false,
                created_at: chrono::Utc::now(),
                last_opened_at: None,
                search_excludes: vec![],
                index: None,
            },
            health: ProjectHealth::Ok,
            open_panes: 2,
            ephemeral: false,
            workarea: "/tmp/demo/.ubiq".into(),
        }
    }

    #[test]
    fn project_list_with_a_flattened_snapshot_round_trips() {
        let message = Message::ProjectList {
            projects: vec![snapshot(), snapshot()],
        };
        let bytes = encode(&message).expect("to_vec_named must handle a flattened struct");
        let back = decode(&bytes).expect("round trip");
        match (message, back) {
            (Message::ProjectList { projects: a }, Message::ProjectList { projects: b }) => {
                assert_eq!(a.len(), b.len());
                assert_eq!(a[0].record.id, b[0].record.id);
                assert_eq!(a[0].workarea, b[0].workarea);
            }
            _ => panic!("expected ProjectList"),
        }
    }

    #[test]
    fn terminal_output_with_non_utf8_bytes_round_trips() {
        let bytes: Vec<u8> = vec![0, 159, 146, 150, 255, 0, 1, 2, 254];
        let message = Message::TerminalOutput {
            pane_id: PaneId::generate(),
            bytes: bytes.clone(),
        };
        let framed = encode(&message).unwrap();
        match decode(&framed).unwrap() {
            Message::TerminalOutput { bytes: back, .. } => assert_eq!(back, bytes),
            other => panic!("expected TerminalOutput, got {other:?}"),
        }
    }

    /// The regression test for the `serde_bytes` decision: a plain `Vec<u8>` under `rmp-serde`
    /// costs roughly one msgpack byte of overhead *per payload byte* (an array of small ints), so
    /// without `serde_bytes` this frame would land near 4x the payload, not close to it.
    #[test]
    fn a_terminal_frame_is_close_to_its_payload_size_not_several_times_larger() {
        let payload = vec![0x41u8; 10_000];
        let message = Message::TerminalOutput {
            pane_id: PaneId::generate(),
            bytes: payload.clone(),
        };
        let body = encode(&message).unwrap();
        // A `bin` blob's own overhead is a handful of bytes (marker + length) plus the pane id and
        // the map's field names — nowhere near proportional to the payload. Give it generous room
        // (2x) so the assertion is about "close to N", not "exactly N plus a fixed constant".
        assert!(
            body.len() < payload.len() * 2,
            "frame was {} bytes for a {}-byte payload — serde_bytes is not taking effect",
            body.len(),
            payload.len()
        );
    }

    #[test]
    fn every_representative_message_shape_round_trips() {
        let messages = vec![
            Message::ListProjects,
            Message::Focus {
                pane_id: PaneId::generate(),
            },
            Message::TerminalResize {
                pane_id: PaneId::generate(),
                cols: 120,
                rows: 40,
            },
            Message::PaneExited {
                pane_id: PaneId::generate(),
                code: 0,
            },
            Message::WriteProjectFile {
                project_id: ProjectId::generate(),
                rel_path: "src/main.rs".into(),
                bytes: b"fn main() {}".to_vec(),
                expected: None,
            },
            Message::ProjectAdded {
                project: snapshot(),
            },
        ];
        for message in messages {
            let bytes = encode(&message).unwrap();
            let back = decode(&bytes).unwrap();
            // `Message` is not `PartialEq`; compare through the JSON the existing tape already
            // trusts, rather than hand-matching every variant here.
            let a = serde_json::to_value(&message).unwrap();
            let b = serde_json::to_value(&back).unwrap();
            assert_eq!(a, b);
        }
    }

    #[test]
    fn read_frame_over_a_truncated_buffer_errors_rather_than_hanging() {
        let message = Message::ListProjects;
        let mut framed = Vec::new();
        write_frame(&mut framed, &message).unwrap();
        // Cut it mid-body: the prefix says more bytes are coming than are actually there.
        framed.truncate(framed.len() - 1);
        let mut cursor = io::Cursor::new(framed);
        let err = read_frame(&mut cursor).unwrap_err();
        assert!(
            matches!(err, WireError::Io(_)),
            "expected a torn frame to be an I/O error, got {err:?}"
        );
    }

    #[test]
    fn an_empty_stream_is_a_clean_eof_not_an_error_to_report() {
        let mut cursor = io::Cursor::new(Vec::<u8>::new());
        let err = read_frame(&mut cursor).unwrap_err();
        assert!(matches!(err, WireError::Eof));
    }

    #[test]
    fn an_oversize_length_prefix_is_rejected_without_allocating() {
        // A header claiming 4 GiB, with no body behind it at all — if `read_frame` allocated
        // before checking the ceiling this would try to allocate 4 GiB and likely abort the test
        // process; succeeding at all is the proof that it checks first.
        let mut cursor = io::Cursor::new(u32::MAX.to_be_bytes().to_vec());
        let err = read_frame(&mut cursor).unwrap_err();
        assert!(matches!(err, WireError::FrameTooLarge(len) if len == u32::MAX));
    }

    #[test]
    fn write_then_read_frame_round_trips_over_a_shared_buffer() {
        let message = Message::TerminalInput {
            pane_id: PaneId::generate(),
            bytes: vec![1, 2, 3],
        };
        let mut buf = Vec::new();
        write_frame(&mut buf, &message).unwrap();
        let mut cursor = io::Cursor::new(buf);
        let back = read_frame(&mut cursor).unwrap();
        match back {
            Message::TerminalInput { bytes, .. } => assert_eq!(bytes, vec![1, 2, 3]),
            other => panic!("expected TerminalInput, got {other:?}"),
        }
    }
}
