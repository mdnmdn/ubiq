//! What Ubiq offers an agent: the built-in servers, their tools, and the argument schemas those
//! tools take.
//!
//! **One table, three readers.** The settings panel's checklist comes from here through
//! [`Message::Mcps`], the `tools/list` a harness asks for comes from here, and
//! [`crate::agent::Agents::compose_run`] asks here whether a name a profile saved is one this
//! build still knows. A server described in two places is a server the panel and the harness can
//! disagree about, and the disagreement would only show as a tool that is offered and then is not
//! there.
//!
//! The schemas are JSON text rather than built values because they are constants: written once,
//! read at the moment a harness asks, and never assembled from anything. Parsing one is
//! [`serde_json`] on a string literal the tests exercise, so a typo is a failing test rather than
//! a tool a harness cannot call.
//!
//! [`Message::Mcps`]: ubiq_proto::messages::Message::Mcps

use serde_json::Value;
use ubiq_proto::mcp::{McpInfo, McpToolInfo};

/// The slug of the wiring proof. Named as a constant because the URL, the catalogue row and the
/// dispatch in [`super::tools`] all have to agree on it.
pub const TEST: &str = "test";

/// The slug of the server that answers "what am I, and where am I working".
pub const PROJECT_INFO: &str = "project-info";

/// One tool, as the catalogue holds it: what the panel shows plus what a harness needs in order
/// to call it.
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// The tool's `inputSchema`, as JSON text. Parsed on the way out.
    pub schema: &'static str,
}

/// One built-in server.
pub struct ServerSpec {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub tools: &'static [ToolSpec],
}

/// Every server this build can inject, in the order a panel lists them.
pub const SERVERS: &[ServerSpec] = &[
    ServerSpec {
        name: TEST,
        title: "Test",
        description: "Proves an agent's MCP wiring reaches Ubiq.",
        tools: &[
            ToolSpec {
                name: "send_notification",
                description: "Raise a Ubiq notification, as this agent.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "text": {"type": "string", "description": "The line to show."},
                        "level": {
                            "type": "string",
                            "enum": ["info", "warning", "error"],
                            "description": "How loudly. Defaults to info."
                        }
                    },
                    "required": ["text"]
                }"#,
            },
            ToolSpec {
                name: "write_log",
                description: "Write a line into Ubiq's own log, tagged with this agent.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "message": {"type": "string", "description": "The line to write."},
                        "level": {
                            "type": "string",
                            "enum": ["debug", "info", "warn", "error"],
                            "description": "Which level to write it at. Defaults to info."
                        }
                    },
                    "required": ["message"]
                }"#,
            },
        ],
    },
    ServerSpec {
        name: PROJECT_INFO,
        title: "Project info",
        description: "Tells the agent what it is and where it is working.",
        tools: &[
            ToolSpec {
                name: "project_info",
                description: "The project this agent was started in: id, name, path and colour.",
                schema: r#"{"type": "object", "properties": {}}"#,
            },
            ToolSpec {
                name: "whoami",
                description: "This agent's own metadata: harness, account, model, mode and cwd.",
                schema: r#"{"type": "object", "properties": {}}"#,
            },
        ],
    },
];

/// The server a slug names, or `None` when this build offers none — which is a 404 on the wire and
/// a dropped name at composition, never an error.
pub fn server(name: &str) -> Option<&'static ServerSpec> {
    SERVERS.iter().find(|spec| spec.name == name)
}

/// Whether this build knows the slug. What [`crate::agent::Agents`] asks before injecting one.
pub fn knows(name: &str) -> bool {
    server(name).is_some()
}

/// The catalogue as the interface is told about it.
pub fn catalogue() -> Vec<McpInfo> {
    SERVERS
        .iter()
        .map(|spec| McpInfo {
            name: spec.name.to_string(),
            title: spec.title.to_string(),
            description: spec.description.to_string(),
            tools: spec
                .tools
                .iter()
                .map(|tool| McpToolInfo {
                    name: tool.name.to_string(),
                    description: tool.description.to_string(),
                })
                .collect(),
        })
        .collect()
}

impl ServerSpec {
    /// This server's tools in MCP's own `tools/list` shape. A schema that will not parse is
    /// reported as the empty object rather than failing the listing: a harness that cannot read
    /// the arguments of one tool must still see the rest.
    pub fn tool_list(&self) -> Vec<Value> {
        self.tools
            .iter()
            .map(|tool| {
                let schema: Value = serde_json::from_str(tool.schema).unwrap_or_else(|error| {
                    tracing::error!(
                        tool = tool.name,
                        "the built-in tool's input schema is not JSON: {error}"
                    );
                    serde_json::json!({"type": "object"})
                });
                serde_json::json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": schema,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_schema_is_json() {
        for spec in SERVERS {
            for tool in spec.tools {
                let parsed: Value = serde_json::from_str(tool.schema)
                    .unwrap_or_else(|error| panic!("{}/{}: {error}", spec.name, tool.name));
                assert_eq!(parsed["type"], "object", "{}/{}", spec.name, tool.name);
            }
        }
    }

    #[test]
    fn the_catalogue_and_the_tool_list_name_the_same_tools() {
        for (info, spec) in catalogue().iter().zip(SERVERS) {
            let listed: Vec<String> = spec
                .tool_list()
                .iter()
                .map(|tool| tool["name"].as_str().unwrap_or_default().to_string())
                .collect();
            let shown: Vec<String> = info.tools.iter().map(|tool| tool.name.clone()).collect();
            assert_eq!(listed, shown, "server {}", spec.name);
        }
    }
}
