//! Knowledge about each supported harness.
//!
//! Where the old design had a `Harness` *struct* of static facts, the target
//! design needs a `Harness` *trait* with behavior: each harness differs in how
//! it is provisioned, launched, and (later) spoken to. See
//! `_docs/target/architecture.md` §"The `Harness` trait" and
//! `_docs/harness/<id>.md` for the curated per-harness notes each impl
//! transcribes.

use std::path::Path;

use crate::spec::{HarnessId, RunSpec};
use crate::Result;

mod claude;
pub use claude::Claude;

/// How to launch the real harness binary after provisioning.
#[derive(Debug, Clone)]
pub struct Launch {
    /// Program to exec, e.g. `"claude"`.
    pub program: String,
    /// Arguments (injected flags first, then the user's passthrough args).
    pub args: Vec<String>,
    /// Environment variables to SET for the child.
    pub env: Vec<(String, String)>,
    /// Environment variables to REMOVE from the inherited env (hygiene).
    pub env_remove: Vec<String>,
}

/// Which I/O modes a harness can support. Only passthrough is used in P1.
#[derive(Debug, Clone, Copy, Default)]
pub struct IoSupport {
    /// Raw tty passthrough (always true).
    pub passthrough: bool,
    /// Claude-style NDJSON stream. (P2)
    pub jsonl: bool,
    /// ACP protocol. (P2/P3)
    pub acp: bool,
}

/// A wrappable agent harness: how to identify, provision, and launch it.
pub trait Harness {
    /// Canonical stable id (e.g. `claude-code`).
    fn id(&self) -> HarnessId;
    /// Human-readable name.
    fn display_name(&self) -> &str;
    /// Launch binary name (e.g. `claude`).
    fn command(&self) -> &str;
    /// Alternate ids/commands that also resolve to this harness.
    fn aliases(&self) -> &[&str];
    /// Populate `dir` (the ephemeral config dir) from `spec`; return how to launch.
    fn provision(&self, spec: &RunSpec, dir: &Path) -> Result<Launch>;
    /// Which I/O modes this harness supports.
    fn io_support(&self) -> IoSupport;
}

/// Every harness `am` knows how to wrap. (P1: Claude Code only; more are
/// added by writing more `Harness` impls.)
pub fn all() -> Vec<Box<dyn Harness>> {
    vec![Box::new(Claude::new())]
}

/// Resolve a harness by id, alias, or launch command (lenient — the CLI boundary).
pub fn resolve(key: &str) -> Option<Box<dyn Harness>> {
    all()
        .into_iter()
        .find(|h| h.id() == key || h.command() == key || h.aliases().contains(&key))
}

/// The list of known harness ids (for error messages).
pub fn known_ids() -> Vec<String> {
    all().iter().map(|h| h.id()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_matches_id_alias_and_command() {
        assert_eq!(resolve("claude-code").unwrap().id(), "claude-code");
        assert_eq!(resolve("claude").unwrap().id(), "claude-code");
        assert!(resolve("nope").is_none());
    }

    #[test]
    fn every_harness_has_a_command() {
        for h in all() {
            assert!(!h.command().is_empty(), "{} missing command", h.id());
        }
    }
}
