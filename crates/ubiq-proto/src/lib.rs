//! The protocol: everything the two halves of Ubiq share, and nothing either half owns alone.
//!
//! The contract is the one piece of Ubiq that is expensive to change, because both halves are
//! written against it and every future topology preserves it. It lives in a crate of its own so
//! that neither half can reach around it: the interface and the host both depend on this, and
//! neither depends on the other.
//!
//! Nothing here draws and nothing here touches disk. A GPUI type in a message, or a path in the
//! bus, is the violation this crate boundary exists to make impossible rather than merely
//! forbidden.
//!
//! - `messages`: the message set, serialisable by construction
//! - `ids`: the contract's identifiers, one newtype per kind
//! - `projects`: the project record, its snapshot, and what the project family carries
//! - `files`: one level of a project's tree, one file's bytes, and what a single path can fail at
//! - `bus`: the switchboard between the one host and the windows attached to it
//! - `log`: the process-wide sink every subsystem writes its diagnostics to

pub mod bus;
pub mod files;
pub mod ids;
pub mod log;
pub mod messages;
pub mod projects;
