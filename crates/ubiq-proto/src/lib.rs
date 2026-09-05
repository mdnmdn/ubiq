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
//! - `connectors`: an authenticated identity at an external service, and the providers there are
//! - `ids`: the contract's identifiers, one newtype per kind
//! - `projects`: the project record, its snapshot, and what the project family carries
//! - `settings`: which half owns a settings blob, and the host's own record
//! - `files`: one level of a project's tree, one file's bytes, and what a single path can fail at
//! - `git`: a project's repository as the host has observed it — overview, working-tree map, errors
//! - `repos`: a repository somewhere else, and the clone that turns one into a project
//! - `work`: a task as it is written down, and the sessions and agents doing it
//! - `conversation`: what a live agent says, in the Agent Client Protocol's vocabulary
//! - `bus`: the switchboard between the one host and the windows attached to it
//! - `log`: the process-wide sink every subsystem writes its diagnostics to

pub mod bus;
pub mod connectors;
pub mod conversation;
pub mod files;
pub mod git;
pub mod ids;
pub mod log;
pub mod messages;
pub mod projects;
pub mod repos;
pub mod search;
pub mod settings;
pub mod work;
