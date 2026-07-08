//! I/O bridging between the local terminal and a running harness.
//!
//! Phase 1 has exactly one mode — raw-tty [`passthrough`] — see
//! `_docs/target/io-modes.md`. Later phases add structured bridges (JSONL,
//! ACP) behind an `IoBridge` trait; this module is where those will live
//! alongside passthrough.

mod passthrough;

pub use passthrough::{pump, RawModeGuard};
