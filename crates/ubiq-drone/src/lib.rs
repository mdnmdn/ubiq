//! The drone: one machine's terminal, its files and its facts, served over a single duplex byte
//! stream.
//!
//! It exists so that a machine can be *used* by an interface running somewhere else without that
//! machine having to be an Ubiq. It knows nothing about agents, harnesses, accounts, profiles,
//! version control or a full-text index — those are the coordinator's, and the coordinator stays
//! where the user is. What is here is the irreducible part: open a pseudo-terminal, move bytes,
//! list a directory, read and write a file, and say what this machine is.
//!
//! - `relay`: the run loop that answers the bus — what stands in for `Coordinator` at a fraction
//!   of its surface, because a drone answers a fraction of the message set
//! - `carrier`: standard input and standard output wired into [`ubiq_host::carrier::pump`]
//!
//! **Everything it links is the lean host** — `ubiq-host` with no default features. `just relay`
//! is the mechanical check that no gated crate reached this tree.
//!
//! It renders nothing, and terminal bytes stay opaque: what leaves here is a `Message` with a pane
//! ID on it, exactly as it would from a local host.

pub mod carrier;
pub mod relay;

/// What a **relay** drone advertises in its [`ubiq_proto::messages::Message::DroneHello`].
///
/// `files` and nothing else, deliberately: this build answers the pane family and the file family,
/// and refuses search, version control, persistence and every harness. Naming a capability it does
/// not have would be worse than naming none — the interface would offer the user something that
/// answers with a refusal. The runtime build, when it exists, is where `harness` is added.
///
/// A set of strings rather than a bitfield, so this list can grow without a schema bump: a peer
/// that has never heard of a name simply does not ask for it.
pub const CAPABILITIES: &[&str] = &["files"];

/// One line of machine facts, for a deployer deciding which binary to send.
///
/// Compile-time constants only: this runs before anything is served, on a machine nobody has
/// established anything about yet, and it must not depend on a sampler thread or a scratch
/// directory existing.
pub fn probe_line() -> String {
    format!(
        "os={} arch={} triplet={}-{}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::env::consts::ARCH,
        std::env::consts::OS,
        std::env::consts::FAMILY,
    )
}
