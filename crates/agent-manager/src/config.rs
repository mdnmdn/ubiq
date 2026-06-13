//! Unified, harness-agnostic configuration model.
//!
//! A user writes a single config describing rules, policies, skills, MCP servers
//! and sub-agents. The [`sync`](crate::sync) module then projects this model
//! onto each concrete harness.

use serde::{Deserialize, Serialize};

/// Top-level unified configuration. Serialized as TOML by default.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UnifiedConfig {
    /// Project-level metadata.
    #[serde(default)]
    pub project: ProjectMeta,

    /// Rules / policies that should apply across harnesses.
    #[serde(default)]
    pub rules: Vec<Rule>,

    /// Reusable skills.
    #[serde(default)]
    pub skills: Vec<Skill>,

    /// MCP server definitions to expose to every harness.
    #[serde(default)]
    pub mcp: Vec<McpServer>,

    /// Sub-agent definitions.
    #[serde(default)]
    pub agents: Vec<Agent>,

    /// Which harnesses are enabled. If empty, all supported harnesses are used.
    #[serde(default)]
    pub harnesses: Vec<String>,
}

/// Free-form project metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectMeta {
    /// Human-readable project name.
    pub name: Option<String>,

    /// Free-form description.
    pub description: Option<String>,
}

/// A rule or policy entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// Stable identifier for this rule.
    pub id: String,

    /// Human-readable title.
    pub title: String,

    /// Path to the rule body (relative to the config file) or inline text.
    pub body: String,
}

/// A reusable skill definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// Stable identifier.
    pub id: String,

    /// Path to the skill's main file (typically a `SKILL.md`).
    pub path: String,
}

/// An MCP server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    /// Stable identifier (e.g. `browser`, `github`).
    pub id: String,

    /// Transport (`stdio`, `sse`, `http`).
    pub transport: McpTransport,

    /// Command and arguments for `stdio` transports.
    #[serde(default)]
    pub command: Option<String>,

    /// Command-line arguments passed to the `stdio` command.
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables to set when launching the server.
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

/// Supported MCP transports.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    /// Local process speaking JSON-RPC over stdio.
    Stdio,
    /// Remote server speaking MCP over Server-Sent Events.
    Sse,
    /// Remote server speaking MCP over plain HTTP.
    Http,
}

/// A sub-agent definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    /// Stable identifier.
    pub id: String,

    /// Path to the agent definition file.
    pub path: String,
}
