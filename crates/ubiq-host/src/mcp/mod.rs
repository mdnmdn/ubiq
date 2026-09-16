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
//! - `kb`: the `ubiq-kb` server, reaching the project's knowledge base through [`crate::kb`] and
//!   its `ops` on the same shape
//!
//! The boundary this sits inside is the ordinary one: nothing here draws. A notification a tool
//! raised goes through [`ubiq_proto::bus::Voice`] as
//! [`ubiq_proto::messages::Message::RaiseNotification`]; a task a tool created, changed or deleted
//! is posted to every window as the same work-family message a click would have produced, so the
//! board redraws without the coordinator answering a question.

pub mod catalogue;
mod kb;
pub mod registry;
pub mod server;
mod tasks;
mod tools;

use std::sync::Arc;

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

/// How the knowledge-base tools reach a project's documents, and how they tell every window what
/// they changed.
///
/// Named for the reach rather than for access, because [`ubiq_proto::kb::KbAccess`] already means
/// something else in this family — whether a source may be written to — and two types spelled the
/// same in one call chain is a confusion nobody should have to hold.
///
/// [`crate::kb::Kb`] is shared rather than copied: it owns the in-memory `Syncing`/`Failed`
/// overrides a fetch writes, so a listener holding its own would answer a state the coordinator
/// has never heard of. Mutations go through [`crate::kb::ops`], which needs no handle at all —
/// only the source and the base path [`crate::kb::Kb`] resolves.
pub struct KbReach {
    pub kb: Arc<crate::kb::Kb>,
    pub everyone: Mailbox,
}
