//! I/O bridging between `am` and a running harness.
//!
//! Two independent things live here (see `_docs/io-modes.md`):
//!
//! - **Core** — the harness-neutral [`model`] ([`AgentInput`], [`AgentEvent`],
//!   [`IoBridge`]) and the piped-spawn helper in [`structured`]. These only
//!   need `serde`/`serde_json`/`std::process`, so they compile under
//!   `--no-default-features` — a lib-mode embedder with neither `pty` nor
//!   `cli` can still build a structured bridge and read events. Concrete
//!   per-harness bridges (NDJSON, JSON-RPC `app-server`, ...) land on top of
//!   this in later steps. The output-side adapters [`acp`] ([`to_acp`]) and
//!   [`agui`] ([`to_agui`]) are core too: stateless, best-effort projections
//!   of [`AgentEvent`] onto the ACP and AG-UI schemas (see each module's
//!   docs for the fidelity caveat).
//! - **`pty`-gated** — raw-tty [`passthrough`], which needs
//!   `crossterm`/`portable-pty` and is Phase 1's only mode.

mod model;
pub use model::{AgentEvent, AgentInput, ApprovalDecision, IoBridge};

pub mod acp;
pub use acp::to_acp;

pub mod agui;
pub use agui::to_agui;

mod structured;
pub use structured::{run_structured, spawn_piped};

mod jsonl;
pub use jsonl::JsonlBridge;

pub mod codex;
pub use codex::CodexBridge;

pub mod opencode;
pub use opencode::OpencodeBridge;

pub mod copilot;
pub use copilot::CopilotBridge;

#[cfg(feature = "pty")]
mod passthrough;
#[cfg(feature = "pty")]
pub use passthrough::{pump, RawModeGuard};
