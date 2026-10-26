//! The Tools dialog's arithmetic: encoders, decoders, converters, generators and a text diff.
//!
//! Named `devtools` because "tools" is already the runnable-tools catalogue (`ubiq_proto::tools`,
//! `ui/tools.rs`); the titlebar button still says **Tools**.
//!
//! **Pure functions only.** Nothing here names GPUI, `AppState`, a path or the host — each module
//! takes text (and, where time matters, a `now` and a zone) and answers with text, so every one is
//! unit-tested in place. The dialog's state is `state/devtools.rs`, its mutators
//! `app/devtools.rs` and its screen `ui/devtools.rs`.

pub mod codec;
pub mod detect;
pub mod diff;
pub mod hash;
pub mod jwt;
pub mod lorem;
pub mod time;
pub mod uuids;
