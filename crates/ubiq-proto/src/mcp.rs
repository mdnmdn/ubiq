//! The MCP servers Ubiq itself offers a harness — not a harness's own `mcpServers` config, but
//! the small set Ubiq injects so an agent can ask about the host it is running under.
//!
//! An [`McpInfo`] is the interface's catalogue row: a server this build can start, named by a
//! slug a profile stores and a launch picks up. What actually answers the protocol — the process,
//! the tool handlers — is host-side and outside this crate on purpose, the same split
//! [`crate::tools::ToolDef`] draws between "what a run is" and "what runs it".

use serde::{Deserialize, Serialize};

/// One tool an [`McpInfo`] server answers, as far as the interface needs to know: enough to list
/// it, never enough to call it — a call is between the harness and the server, and Ubiq is not on
/// that path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpToolInfo {
    /// The tool's own name, as the server will register it.
    pub name: String,
    /// One line saying what it does, for the settings panel's list.
    pub description: String,
}

/// One MCP server Ubiq can inject into a harness, as the interface is told about it.
///
/// `name` is the URL slug (`test`, `project-info`) — it is both how the interface asks a running
/// host to start one and the id a [`crate::messages::ProfileInfo::mcps`] entry stores, so it
/// never changes once a profile has saved it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpInfo {
    /// The slug: stable, and what a profile or a start names.
    pub name: String,
    /// What the settings panel and the start form show instead of the slug.
    pub title: String,
    /// One paragraph saying what this server is for, the way [`crate::tools::ToolDef`]'s
    /// neighbours are documented.
    pub description: String,
    /// The tools this server answers, for a panel that lists them under the server's own row.
    pub tools: Vec<McpToolInfo>,
}
