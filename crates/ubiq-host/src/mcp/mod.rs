//! The MCP surface Ubiq exposes to the agents it hosts.
//!
//! An agent Ubiq starts knows nothing about Ubiq. It has a folder and a harness and no way to ask
//! what it is, where it is working, or how to tell the person watching that something happened.
//! This module is the answer: a small set of MCP servers Ubiq injects into a run, so the harness
//! reaches back through the one protocol every harness already speaks.
//!
//! ```text
//! one loopback port, bound once at startup
//!   POST /mcps/<agent-or-pane-id>/<mcp-name>
//!                    │              └── catalogue: which built-in answers
//!                    └── registry:  which agent is asking
//! ```
//!
//! **One server for every agent, not one server per agent.** A harness is handed a URL, and a URL
//! is the cheapest possible identity: the port is bound once, the listener holds no per-agent
//! state, and starting an agent costs a row in a map rather than a thread and a socket. Nothing is
//! remembered between two requests — the path says who is calling on every call — which is what
//! makes the whole of the lifecycle "register at launch, forget at retirement".
//!
//! - `catalogue`: which servers this build offers and what tools each answers — the one table the
//!   settings panel, `tools/list` and the composer all read
//! - `registry`: what the host knows about each running agent, keyed by the segment its URL carries
//! - `server`: the listener, the routing and the JSON-RPC
//! - `tools`: what the built-in tools actually do
//! - `tasks`: the `manage-ubiq-tasks` server, talking to [`crate::work::Work`] through a shared
//!   handle
//!
//! The boundary this sits inside is the ordinary one: nothing here draws. A notification a tool
//! raised goes through [`ubiq_proto::bus::Voice`] as
//! [`ubiq_proto::messages::Message::RaiseNotification`]; a task a tool created, changed or deleted
//! is posted to every window as the same work-family message a click would have produced, so the
//! board redraws without the coordinator answering a question.

pub mod catalogue;
pub mod registry;
pub mod server;
mod tasks;
mod tools;

use ubiq_proto::bus::Mailbox;

use crate::work;

pub use catalogue::{catalogue, knows};
pub use registry::{AgentFacts, ProjectFacts, Registry};
pub use server::{Serving, start};

/// How the task tools reach the board, and how they tell every window what they changed.
///
/// Held by the listener for the life of the process. The handle is the same [`work::Handle`] the
/// coordinator uses, so an agent and a window never disagree about what the file holds.
pub struct WorkAccess {
    pub work: work::Handle,
    pub everyone: Mailbox,
}
