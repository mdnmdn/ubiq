//! An annotated markdown document, read or written whole: a task's plan, or a file the project's
//! own tree holds.
//!
//! **Which one is a [`Target`], and nothing below that type cares.** The wire names a
//! [`DocumentHandle`]; the coordinator resolves it here, once, into the paths the body and the
//! sidecar occupy; the block matcher, the orphaning rule, the conflict arbitration and the
//! provenance layer are then written once and serve both. That is the whole of `D161` in code — a
//! second matcher for a second kind of document is the failure this shape exists to prevent.
//!
//! **A plan belongs to any task carrying a [`ubiq_proto::work::Level`]** — not to ordinary tasks,
//! and not only to some special mission subtype. Every method here that reads or writes a plan
//! checks that first, against [`crate::work::Work`] through the same [`crate::work::Handle`] the
//! coordinator and the MCP listener already share, and refuses with [`Message::PlanError`] where
//! it does not hold — the same posture [`crate::work::Work::parent_refusal`] takes with a parent
//! that is not itself allowed to be one.
//!
//! [`Handle`] mirrors [`crate::work::Handle`] on purpose: one project's plans are cheap enough that
//! no in-memory cache sits in front of [`crate::store::plan::FilePlanStore`] the way `Work` caches
//! tasks — a plan is read or written on request, never on every keystroke — but the sharing shape
//! (an `Arc<Mutex<_>>` clone held by the coordinator and by the MCP listener's `ubiq-plan` server)
//! is the same, for the same reason: an agent and a window must never disagree about what the file
//! holds.
//!
//! **The MCP reach type holds the plan store, not this module's [`Handle`] paired with a work
//! handle of its own** — `crate::mcp::PlanReach` carries only [`Handle`]; the level check still
//! happens, because [`Plans`] itself holds `crate::work::Handle` internally. That is what the
//! staging card asks for: the *reach struct* an agent's tool call is handed stays a plan store and
//! nothing else, while the service behind it is free to depend on `Work` the way any other
//! in-process collaborator would.
//!
//! **Annotations hang off blocks, and the blocks are re-matched on every save.** The body on disk
//! stays plain markdown with nothing of Ubiq's in it; [`blocks`] holds the id assignment, and this
//! module is what calls it — a save re-indexes the document, carries the ids it can, and flags
//! every annotation whose block is gone as orphaned rather than deleting it. **Anyone may resolve
//! an annotation, including an agent**, so there is no author check anywhere below.

pub mod blocks;
pub mod lines;
pub mod provenance;

/// `Plans`, `Handle`, `Target` and `Saver` — the part of this module that checks a task's level
/// through `crate::work::Handle`, which only exists behind `harness`. `blocks`, `lines` and
/// `provenance` above have no such dependency, which is why they stay reachable without it —
/// `store/plan.rs` reads a plan's sidecar with neither the coordinator nor an agent's harness
/// running.
#[cfg(feature = "harness")]
mod service;
#[cfg(feature = "harness")]
pub use service::*;
