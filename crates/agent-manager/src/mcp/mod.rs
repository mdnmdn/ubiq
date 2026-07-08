//! In-process MCP for lib mode.
//!
//! A library embedder (e.g. the Ubiq Tauri app) can register a tool the
//! wrapped agent calls without spawning a subprocess or standing up its own
//! MCP server. The embedder implements [`McpService`] and wraps it in an
//! [`crate::spec::InProcessMcpHandle`] on [`crate::spec::McpRef::InProcess`].
//!
//! This module is split in two:
//! - The trait + types here (`ToolDef`, `McpService`) are core — always
//!   compiled, no extra deps — so `spec`/`RunSpec` can reference them
//!   regardless of feature flags.
//! - [`server`] (behind the `inproc-mcp` feature) hosts an [`McpService`] on
//!   a loopback HTTP MCP endpoint. `provision()` uses it to turn an
//!   `McpRef::InProcess` into a normal `McpRef::Inline` http server before
//!   handing the spec to the harness — see `_docs` for the mechanism.

#[cfg(feature = "inproc-mcp")]
pub mod server;

/// One tool an in-process [`McpService`] exposes, in the shape MCP's
/// `tools/list` expects (`name`/`description`/`inputSchema`).
#[derive(Debug, Clone)]
pub struct ToolDef {
    /// Tool name, as the agent will call it.
    pub name: String,
    /// Human-readable description shown to the agent.
    pub description: String,
    /// JSON Schema describing the tool's `arguments` shape.
    pub input_schema: serde_json::Value,
}

/// An embedder-provided in-process MCP tool set.
///
/// Implementations must be `Send + Sync`: the `inproc-mcp` HTTP server calls
/// through a shared `Arc<dyn McpService>` from its serving thread.
pub trait McpService: Send + Sync {
    /// The tools this service exposes (served from `tools/list`).
    fn tools(&self) -> Vec<ToolDef>;

    /// Invoke tool `name` with `arguments`.
    ///
    /// `Err` is reported as an in-band MCP tool error (`isError: true`) with
    /// the error's message as the tool's text output — MCP convention is
    /// that tool failures are not JSON-RPC-level errors.
    fn call(&self, name: &str, arguments: serde_json::Value) -> crate::Result<serde_json::Value>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A stub service with a single `echo` tool, shared by these unit tests
    /// and (via a copy in `server`'s tests / `tests/inproc_mcp.rs`) the
    /// feature-gated E2E tests.
    struct EchoService;

    impl McpService for EchoService {
        fn tools(&self) -> Vec<ToolDef> {
            vec![ToolDef {
                name: "echo".to_string(),
                description: "Echoes its arguments back.".to_string(),
                input_schema: json!({"type": "object"}),
            }]
        }

        fn call(&self, name: &str, arguments: serde_json::Value) -> crate::Result<serde_json::Value> {
            if name != "echo" {
                anyhow::bail!("unknown tool: {name}");
            }
            Ok(json!({ "echoed": arguments }))
        }
    }

    #[test]
    fn tools_lists_the_echo_tool_with_its_schema() {
        let svc = EchoService;
        let tools = svc.tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
        assert_eq!(tools[0].input_schema, json!({"type": "object"}));
    }

    #[test]
    fn call_echoes_arguments_back() {
        let svc = EchoService;
        let result = svc.call("echo", json!({"x": 1})).unwrap();
        assert_eq!(result, json!({"echoed": {"x": 1}}));
    }

    #[test]
    fn call_unknown_tool_is_an_error() {
        let svc = EchoService;
        let err = svc.call("nope", json!({})).unwrap_err();
        assert!(err.to_string().contains("unknown tool"));
    }
}
