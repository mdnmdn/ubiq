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
//! - `mission`: the `ubiq-mission` / `use-mission` pair, reaching a mission's record, journal and
//!   documents through [`crate::mission::Missions`] on the same shape — the `manage` / `use` split
//!   the task servers already have, and the mission is resolved from who is calling rather than
//!   from an argument
//! - `plan`: the `ubiq-plan` server, reading and writing a task's plan through
//!   [`crate::plan::Plans`] on the same shape — its own server rather than more tools on
//!   `manage-ubiq-tasks`, so that one keeps a name that still describes it
//! - `kb`: the `ubiq-kb` server, reaching the project's knowledge base through [`crate::kb`] and
//!   its `ops` on the same shape
//! - `help`: the `ubiq-help` server, reaching Ubiq's own documentation through [`crate::help`] —
//!   an agent's read of the same manual a person opens with the `?` in the titlebar
//! - `ask`: the `ubiq-ask` server, the one tool that does not answer itself — it parks on
//!   [`crate::ask::Asks`] until a person answers, on a thread of its own so the listener stays
//!   free (`D138`)
//!
//! The boundary this sits inside is the ordinary one: nothing here draws. A notification a tool
//! raised goes through [`ubiq_proto::bus::Voice`] as
//! [`ubiq_proto::messages::Message::RaiseNotification`]; a task a tool created, changed or deleted
//! is posted to every window as the same work-family message a click would have produced, so the
//! board redraws without the coordinator answering a question.

mod ask;
pub mod catalogue;
mod help;
mod kb;
mod mission;
mod plan;
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
    /// How a board change reaches the host's own run loop.
    ///
    /// **The mission scheduler is why this is here.** It runs on the coordinator's thread with no
    /// timer, and `use-task::change_state` — a worker saying it has finished — happens on this
    /// listener's thread instead. The tool says
    /// [`ubiq_proto::messages::Message::MissionSchedule`] into the host's own inbox and the run
    /// loop does the deciding, exactly as `ubiq-ask` says
    /// [`ubiq_proto::messages::Message::AskUser`] (`D138`). It names the task, not the mission:
    /// nothing here has to know whether the task is in one.
    pub wake: ubiq_proto::bus::Voice,
}

/// How the `ubiq-plan` tools reach a task's plan, and how they tell every window what changed.
///
/// **Just the plan store, deliberately.** The level check every plan operation needs — a plan
/// belongs to any task carrying a [`ubiq_proto::work::Level`], not to an ordinary task — happens
/// inside [`crate::plan::Plans`] itself, which holds its own [`work::Handle`] for exactly that.
/// The tool handlers in [`super::plan`] never see a work handle at all: they call
/// [`crate::plan::Plans::load`] / [`crate::plan::Plans::save`] and read the refusal, if any, back
/// out of the [`crate::reply::Reply`] list those already return. On the decision recorded in
/// `_docs/wip/planning-system.md` — `PlanReach` holds the plan store, not the work handle.
pub struct PlanReach {
    pub plans: crate::plan::Handle,
    pub everyone: Mailbox,
}

/// How the mission tools reach the mission the caller is in, and how they tell every window what
/// they changed.
///
/// **Just the missions**, on [`PlanReach`]'s own footing and for its reason: the anchor-task check
/// every mission operation needs lives inside [`crate::mission::Missions`], which holds its own
/// [`work::Handle`] for exactly that. The three tools that reach past the mission —
/// `create_mission_task`, `list_agents` and `message_agent` for the board, `write_document` and
/// `read_document` for the plan store — are handed the [`WorkAccess`] and [`PlanReach`] the
/// listener already carries rather than a second copy of either kept here.
pub struct MissionReach {
    pub missions: crate::mission::Handle,
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

/// How the help tools reach Ubiq's own documentation.
///
/// No `everyone` mailbox: nothing a help tool does changes anything a window would need to
/// re-draw for — every tool here only reads a catalogue [`crate::help::Help`] has already
/// resolved, on the same standing [`crate::help`]'s module doc gives for why that resolution
/// itself needs no thread either. `Arc` for the reason [`KbReach`]'s does: the coordinator holds
/// one clone, the listener thread another, and both must see the one cached outcome.
pub struct HelpReach {
    pub help: Arc<crate::help::Help>,
}

/// How the ask tool parks a call until a person answers it.
///
/// One field, and it is the table itself: the question goes out to the coordinator through the
/// [`ubiq_proto::bus::Voice`] the listener already holds — the same route a notification takes —
/// so nothing extra is needed to *raise* an ask, only somewhere to hold it while it waits. `Arc`
/// for the reason [`KbReach`]'s is: the coordinator answers into the same table the serving
/// thread is parked on, and two copies would be two different waits.
pub struct AskReach {
    pub asks: Arc<crate::ask::Asks>,
}
