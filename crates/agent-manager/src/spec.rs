//! Fully-resolved, harness-agnostic description of one agent run.
//!
//! This module defines [`RunSpec`], the boundary type between "figure out what to run"
//! and "run it". It contains no file I/O, no clap, no async — just pure types and
//! a constructor for tests and in-memory usage.

use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use crate::account::Account;
use crate::config::McpServer;

/// Stable, lowercase harness identifier (e.g. `claude-code`).
pub type HarnessId = String;

/// Account / credential profile id — the [`crate::account::AccountStore`] key.
pub type AccountId = String;

/// A resolved skill to inject: its id and the on-disk folder to materialize.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillRef {
    /// Catalog skill id.
    pub id: String,
    /// Path to the skill folder (contains `SKILL.md`).
    pub path: PathBuf,
}

/// A resolved MCP server to inject.
#[derive(Debug, Clone)]
pub enum McpRef {
    /// A catalog entry resolved by id.
    Catalog(McpServer),
    /// An inline definition (lib mode or `--mcp-json`).
    Inline(McpServer),
    /// An in-process server hosted by the embedding program (lib mode only).
    ///
    /// Consumed by `provision()` when the `inproc-mcp` feature is on: it
    /// starts a loopback HTTP MCP server backed by
    /// [`InProcessMcpHandle::service`] and replaces this entry with an
    /// [`McpRef::Inline`] `http` server before handing the spec to the
    /// harness. When the feature is off, harnesses `bail!` on this variant.
    InProcess(InProcessMcpHandle),
}

/// Handle to an in-process MCP server: a logical name plus the embedder's
/// [`crate::mcp::McpService`] implementation backing it.
#[derive(Clone)]
pub struct InProcessMcpHandle {
    /// Logical name of the in-process server (becomes the MCP server id).
    pub name: String,
    /// The embedder-provided tool implementation.
    pub service: std::sync::Arc<dyn crate::mcp::McpService>,
}

impl std::fmt::Debug for InProcessMcpHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InProcessMcpHandle")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// A hook to wire into the harness's native hook slots. Placeholder until P2/P3.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct HookRef {
    /// Hook id.
    pub id: String,
}

/// Always-on instructions / first prompt to seed.
#[derive(Debug, Clone)]
pub struct Instructions {
    /// Always-on instructions (written to the harness's memory file, e.g. CLAUDE.md).
    pub instructions: Option<String>,
    /// An initial prompt to send/seed for the run.
    pub prompt: Option<String>,
}

impl Instructions {
    /// Check whether both instructions and prompt are empty.
    pub fn is_empty(&self) -> bool {
        self.instructions.is_none() && self.prompt.is_none()
    }
}

/// Where the ephemeral config dir lives and whether to keep it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConfigStrategy {
    /// A throwaway dir created per run and removed on exit (default).
    #[default]
    Ephemeral,
    /// A fixed dir the caller chose (kept after the run; for debugging).
    Fixed(PathBuf),
}

/// Sandbox settings (isol8). Off by default. Placeholder until P3.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub enum Isolation {
    /// No sandbox (default).
    #[default]
    None,
    /// Run inside the named isol8 profile.
    Sandboxed(String),
}

/// How `am` talks to the agent and exposes it outward.
///
/// `#[serde(rename_all = "snake_case")]` so this round-trips as
/// `"passthrough"` / `"structured"` (matching the CLI's `--io` values).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IoModes {
    /// Forward the tty verbatim (default, Phase 1).
    #[default]
    Passthrough,
    /// Drive the agent over a harness-neutral [`crate::io::IoBridge`]
    /// (Phase 2+). See `_docs/target/io-modes.md`.
    Structured,
}

/// A permission/policy preset (what `--safe` expands to). Rendered per-harness
/// by the provisioner (e.g. Claude Code `settings.json` `permissions`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Policy {
    /// e.g. "restricted" / "acceptEdits" / "bypassPermissions".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    /// Allowed tool rules.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Ask-first tool rules.
    #[serde(default)]
    pub ask: Vec<String>,
    /// Denied tool rules.
    #[serde(default)]
    pub deny: Vec<String>,
}

/// Everything needed to launch one agent run. Harness-agnostic.
#[derive(Debug, Clone)]
pub struct RunSpec {
    /// Which harness to wrap.
    pub harness: HarnessId,
    /// Resolved skills to inject.
    pub skills: Vec<SkillRef>,
    /// Resolved MCP servers to inject.
    pub mcps: Vec<McpRef>,
    /// Hooks to wire in. (P2/P3)
    pub hooks: Vec<HookRef>,
    /// Resolved account/credential reference, if any. Holds only references
    /// (env-var names, a base URL, a helper command, a private home dir) —
    /// never a secret value; see [`Account`].
    pub account: Option<Account>,
    /// Resolved permission/policy preset (from `--safe`), if any.
    pub policy: Option<Policy>,
    /// Always-on instructions / first prompt. (P2)
    pub initial: Option<Instructions>,
    /// Ephemeral vs fixed config dir.
    pub config: ConfigStrategy,
    /// Sandbox settings. (P3)
    pub isolation: Isolation,
    /// I/O mode. Default passthrough.
    pub io: IoModes,
    /// Verbatim extra args forwarded to the harness binary.
    pub passthrough_args: Vec<String>,
    /// Working directory for the agent.
    pub cwd: PathBuf,
}

impl RunSpec {
    /// Create a minimal spec for `harness` running in `cwd`, everything else defaulted
    /// (no skills/mcps/hooks/account, ephemeral config, no isolation, passthrough I/O).
    pub fn new(harness: HarnessId, cwd: PathBuf) -> Self {
        Self {
            harness,
            skills: Vec::new(),
            mcps: Vec::new(),
            hooks: Vec::new(),
            account: None,
            policy: None,
            initial: None,
            config: ConfigStrategy::Ephemeral,
            isolation: Isolation::None,
            io: IoModes::Passthrough,
            passthrough_args: Vec::new(),
            cwd,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_spec_new_defaults() {
        let harness = "claude-code".to_string();
        let cwd = PathBuf::from("/tmp/test");
        let spec = RunSpec::new(harness.clone(), cwd.clone());

        assert_eq!(spec.harness, harness);
        assert_eq!(spec.cwd, cwd);
        assert!(spec.skills.is_empty());
        assert!(spec.mcps.is_empty());
        assert!(spec.hooks.is_empty());
        assert!(spec.account.is_none());
        assert!(spec.policy.is_none());
        assert!(spec.initial.is_none());
        assert_eq!(spec.config, ConfigStrategy::Ephemeral);
        assert!(matches!(spec.isolation, Isolation::None));
        assert_eq!(spec.io, IoModes::Passthrough);
        assert!(spec.passthrough_args.is_empty());
    }

    #[test]
    fn test_serde_skill_ref() {
        let skill = SkillRef {
            id: "my-skill".to_string(),
            path: PathBuf::from("/path/to/skill"),
        };

        let json = serde_json::to_string(&skill).expect("serialize");
        let deserialized: SkillRef = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(skill, deserialized);
    }

    #[test]
    fn test_serde_config_strategy_fixed() {
        let strategy = ConfigStrategy::Fixed(PathBuf::from("/home/user/.config"));
        let json = serde_json::to_string(&strategy).expect("serialize");
        let deserialized: ConfigStrategy = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(strategy, deserialized);
    }

    #[test]
    fn test_serde_io_modes() {
        let mode = IoModes::Passthrough;
        let json = serde_json::to_string(&mode).expect("serialize");
        let deserialized: IoModes = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(mode, deserialized);
    }
}
