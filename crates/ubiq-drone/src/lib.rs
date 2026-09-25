//! The drone: one machine's terminal, its files and its facts, served over a single duplex byte
//! stream.
//!
//! It exists so that a machine can be *used* by an interface running somewhere else without that
//! machine having to be an Ubiq. It knows nothing about agents, harnesses, accounts, definitions,
//! version control or a full-text index — those are the coordinator's, and the coordinator stays
//! where the user is. What is here is the irreducible part: open a pseudo-terminal, move bytes,
//! list a directory, read and write a file, and say what this machine is.
//!
//! - `relay`: the run loop that answers the bus — what stands in for `Coordinator` at a fraction
//!   of its surface, because a drone answers a fraction of the message set
//! - `carrier`: standard input and standard output wired into [`ubiq_host::carrier::pump`]
//! - `search`: shelling out to `rg`, `ag` or `grep` for the search family — see its own doc for why
//!   a drone never builds an index instead
//! - `socket`: the unix socket a **detached** drone is found again through, and the byte relay
//!   that attaches to one
//! - `linger`: how long a drone with no client waits before it kills its panes and goes
//! - `scrollback`: where a pane's output goes while nobody is attached to see it
//! - `state`: the state file beside a listening drone's socket, and how a caller finds, lists and
//!   prunes them
//!
//! **Everything it links is the lean host** — `ubiq-host` with no default features. `just relay`
//! is the mechanical check that no gated crate reached this tree.
//!
//! It renders nothing, and terminal bytes stay opaque: what leaves here is a `Message` with a pane
//! ID on it, exactly as it would from a local host.

pub mod carrier;
pub mod linger;
pub mod relay;
pub mod scrollback;
pub mod search;
pub mod socket;
pub mod state;

/// What a **relay** drone advertises in its [`ubiq_proto::messages::Message::DroneHello`].
///
/// `files` always, plus `harness`less version control's absence and persistence's stay refused —
/// this build has no git library and no keychain of its own. `search` is named only when
/// [`search::probe`] actually found a tool on this machine's `PATH`: naming a capability this
/// drone would only refuse is worse than naming none at all, since the interface would offer the
/// user something that answers with a refusal every time. That is also why this is a function and
/// not a `const` — whether a tool exists is a fact about the machine the binary happens to be
/// running on, discovered once and cached in [`search::probe`], not compiled in.
///
/// A set of strings rather than a bitfield, so this list can grow without a schema bump: a peer
/// that has never heard of a name simply does not ask for it.
pub fn capabilities() -> Vec<&'static str> {
    let mut names = vec!["files"];
    if search::probe().is_some() {
        names.push("search");
    }
    names
}

/// This build's triple: architecture, OS and libc family, compile-time and nothing else.
///
/// Shared between [`probe_line`], for a deployer choosing a binary, and [`state::DroneState`], for
/// a caller telling two drones' builds apart without connecting to either.
pub fn triple() -> String {
    format!(
        "{}-{}-{}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        std::env::consts::FAMILY,
    )
}

/// One line of machine facts, for a deployer deciding which binary to send.
///
/// Compile-time constants only: this runs before anything is served, on a machine nobody has
/// established anything about yet, and it must not depend on a sampler thread or a scratch
/// directory existing.
pub fn probe_line() -> String {
    format!(
        "os={} arch={} triplet={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        triple(),
    )
}
