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

/// The slug of the server that reads and writes this project's tasks.
pub const MANAGE_UBIQ_TASKS: &str = "manage-ubiq-tasks";

/// The slug of the thinner server an agent uses to look up a task, move it, and leave a comment.
pub const USE_TASK: &str = "use-task";

/// The slug of the server that reads and writes this project's knowledge base.
pub const UBIQ_KB: &str = "ubiq-kb";

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

/// The todo (sub-task) tools: written once and shared between `manage-ubiq-tasks` and `use-task`,
/// which both route them to the same handlers in `tasks.rs`.
const ADD_TODO: ToolSpec = ToolSpec {
    name: "add_todo",
    description: "Append a todo (sub-task) to a task.",
    schema: r#"{
        "type": "object",
        "properties": {
            "task_id": {"type": "string"},
            "title": {"type": "string"}
        },
        "required": ["task_id", "title"]
    }"#,
};
const UPDATE_TODO: ToolSpec = ToolSpec {
    name: "update_todo",
    description: "Patch a todo. Null or omitted fields are left alone. Set done to true to mark it done, or false to return it to idle.",
    schema: r#"{
        "type": "object",
        "properties": {
            "task_id": {"type": "string"},
            "todo_id": {"type": "string"},
            "title": {"type": "string"},
            "done": {"type": "boolean"}
        },
        "required": ["task_id", "todo_id"]
    }"#,
};
const DELETE_TODO: ToolSpec = ToolSpec {
    name: "delete_todo",
    description: "Remove a todo from a task.",
    schema: r#"{
        "type": "object",
        "properties": {
            "task_id": {"type": "string"},
            "todo_id": {"type": "string"}
        },
        "required": ["task_id", "todo_id"]
    }"#,
};

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
    ServerSpec {
        name: MANAGE_UBIQ_TASKS,
        title: "Manage Ubiq tasks",
        description: "The current project's task board: overview, search, tags, tasks, todos and comments.",
        tools: &[
            ToolSpec {
                name: "overview",
                description: "Every status and tag, and for each status the count and the first 10 task summaries. Pass categories to include only those columns.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "categories": {
                            "type": "array",
                            "items": {
                                "type": "string",
                                "enum": ["backlog", "ready", "blocked", "in progress", "in review", "done", "abandoned"]
                            },
                            "description": "Board columns to include. Omit, null or empty for all."
                        }
                    }
                }"#,
            },
            ToolSpec {
                name: "search_tasks",
                description: "Find tasks by optional text (title, description, key, tags, todos, comments), status, tags and priority. Status and tag filters are OR within each list.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "text": {"type": "string", "description": "Case-insensitive substring. Omit or null to skip."},
                        "statuses": {
                            "type": "array",
                            "items": {
                                "type": "string",
                                "enum": ["backlog", "ready", "blocked", "in progress", "in review", "done", "abandoned"]
                            },
                            "description": "Match any of these columns. Omit, null or empty for all."
                        },
                        "labels": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Match any of these tag names. Omit, null or empty for all."
                        },
                        "priorities": {
                            "type": "array",
                            "items": {"type": "string", "enum": ["low", "normal", "high"]},
                            "description": "Match any of these priorities. Omit, null or empty for all."
                        },
                        "maxRows": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "How many tasks to return. Defaults to 10, capped at 100."
                        }
                    }
                }"#,
            },
            ToolSpec {
                name: "list_tags",
                description: "The tags this project knows: those on its cards, plus any created this session that no card has used yet.",
                schema: r#"{"type": "object", "properties": {}}"#,
            },
            ToolSpec {
                name: "create_tag",
                description: "Remember a tag for this project. A name that already exists is returned as it is. Unused tags last until the host restarts; putting one on a task makes it durable.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "The tag's name."},
                        "colour": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "Swatch index. Defaults to 0."
                        }
                    },
                    "required": ["name"]
                }"#,
            },
            ToolSpec {
                name: "create_task",
                description: "Create a task on this project's board. Status defaults to backlog; omitted fields stay unset.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "title": {"type": "string", "description": "The card's title."},
                        "description": {"type": "string"},
                        "status": {
                            "type": "string",
                            "enum": ["backlog", "ready", "blocked", "in progress", "in review", "done", "abandoned"]
                        },
                        "priority": {"type": "string", "enum": ["low", "normal", "high"]},
                        "kind": {"type": "string", "enum": ["bug", "feature", "chore", "docs"]},
                        "key": {"type": "string", "description": "Human id, e.g. UBQ-123."},
                        "link": {"type": "string"},
                        "labels": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Tag names to put on the card."
                        }
                    },
                    "required": ["title"]
                }"#,
            },
            ToolSpec {
                name: "update_task",
                description: "Patch a task. Null or omitted fields are left alone. labels replaces the whole set — send every tag the card should have.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "task_id": {"type": "string"},
                        "title": {"type": "string"},
                        "description": {"type": "string"},
                        "status": {
                            "type": "string",
                            "enum": ["backlog", "ready", "blocked", "in progress", "in review", "done", "abandoned"]
                        },
                        "priority": {"type": "string", "enum": ["low", "normal", "high"]},
                        "kind": {"type": "string", "enum": ["bug", "feature", "chore", "docs"]},
                        "key": {"type": "string"},
                        "link": {"type": "string"},
                        "labels": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "The full tag set. Omit or null to leave tags alone; [] clears them."
                        }
                    },
                    "required": ["task_id"]
                }"#,
            },
            ToolSpec {
                name: "delete_task",
                description: "Delete a task. Its todos go with it.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "task_id": {"type": "string"}
                    },
                    "required": ["task_id"]
                }"#,
            },
            ADD_TODO,
            UPDATE_TODO,
            DELETE_TODO,
            ToolSpec {
                name: "get_task",
                description: "One task whole: fields, todos and comments.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "task_id": {"type": "string"}
                    },
                    "required": ["task_id"]
                }"#,
            },
            ToolSpec {
                name: "add_comment",
                description: "Leave a comment on a task. The author is stamped as this agent.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "task_id": {"type": "string"},
                        "text": {"type": "string"}
                    },
                    "required": ["task_id", "text"]
                }"#,
            },
        ],
    },
    ServerSpec {
        name: USE_TASK,
        title: "Use task",
        description: "Look up a task, move it along the board, and leave a comment.",
        tools: &[
            ToolSpec {
                name: "search_tasks",
                description: "Find tasks by optional text, status, tags and priority. Status and tag filters are OR within each list.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "text": {"type": "string", "description": "Case-insensitive substring. Omit or null to skip."},
                        "statuses": {
                            "type": "array",
                            "items": {
                                "type": "string",
                                "enum": ["backlog", "ready", "blocked", "in progress", "in review", "done", "abandoned"]
                            },
                            "description": "Match any of these columns. Omit, null or empty for all."
                        },
                        "labels": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Match any of these tag names. Omit, null or empty for all."
                        },
                        "priorities": {
                            "type": "array",
                            "items": {"type": "string", "enum": ["low", "normal", "high"]},
                            "description": "Match any of these priorities. Omit, null or empty for all."
                        },
                        "maxRows": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "How many tasks to return. Defaults to 10, capped at 100."
                        }
                    }
                }"#,
            },
            ToolSpec {
                name: "get_task",
                description: "One task whole: fields, todos and comments.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "task_id": {"type": "string"}
                    },
                    "required": ["task_id"]
                }"#,
            },
            ToolSpec {
                name: "change_state",
                description: "Move a task to another board column.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "task_id": {"type": "string"},
                        "status": {
                            "type": "string",
                            "enum": ["backlog", "ready", "blocked", "in progress", "in review", "done", "abandoned"]
                        }
                    },
                    "required": ["task_id", "status"]
                }"#,
            },
            ToolSpec {
                name: "add_comment",
                description: "Leave a comment on a task. The author is stamped as this agent.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "task_id": {"type": "string"},
                        "text": {"type": "string"}
                    },
                    "required": ["task_id", "text"]
                }"#,
            },
            ADD_TODO,
            UPDATE_TODO,
            DELETE_TODO,
        ],
    },
    ServerSpec {
        name: UBIQ_KB,
        title: "Project documents",
        description: "The current project's knowledge base: its sources, and the documents in them, read and written by name.",
        tools: &[
            ToolSpec {
                name: "list_kb_sources",
                description: "Every knowledge-base source of this project: its name, id, kind, origin, whether it is writable, its filter and its state. Call this first — the names it gives are what every other tool's path starts with.",
                schema: r#"{"type": "object", "properties": {}}"#,
            },
            ToolSpec {
                name: "list_kb_documents",
                description: "One folder's entries. Each comes back with the address to pass to the other tools.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "A folder, as <source>/path/to/folder. A bare <source> is that source's top level. <source> is the name list_kb_sources gave, or its id."
                        }
                    },
                    "required": ["path"]
                }"#,
            },
            ToolSpec {
                name: "read_kb_document",
                description: "A document's text.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The document, as <source>/path/to/file.md."
                        }
                    },
                    "required": ["path"]
                }"#,
            },
            ToolSpec {
                name: "write_kb_document",
                description: "Write a document's whole contents, creating it if the folder holding it exists. Refused for a source that is not writable.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The document, as <source>/path/to/file.md. Its parent folder must already exist."
                        },
                        "contents": {"type": "string", "description": "The document's whole new text."}
                    },
                    "required": ["path", "contents"]
                }"#,
            },
            ToolSpec {
                name: "create_kb_entry",
                description: "Create an empty document or a folder. Refused when something is already there, and for a source that is not writable.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "What to create, as <source>/path/to/entry. Its parent folder must already exist."
                        },
                        "kind": {
                            "type": "string",
                            "enum": ["file", "folder"],
                            "description": "Whether to create an empty document or a folder."
                        }
                    },
                    "required": ["path", "kind"]
                }"#,
            },
            ToolSpec {
                name: "rename_kb_entry",
                description: "Rename one document or folder in place. Refused for a source that is not writable.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The entry to rename, as <source>/path/to/entry."
                        },
                        "new_name": {
                            "type": "string",
                            "description": "The new leaf name. A name, never a path: it may not contain a separator."
                        }
                    },
                    "required": ["path", "new_name"]
                }"#,
            },
            ToolSpec {
                name: "delete_kb_entry",
                description: "Delete one document, or a folder and everything under it. Refused for a source that is not writable.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The entry to delete, as <source>/path/to/entry."
                        }
                    },
                    "required": ["path"]
                }"#,
            },
            ToolSpec {
                name: "sync_kb_source",
                description: "Fetch or refresh a git source. It runs in the background; call list_kb_sources again to see how it ended. A folder or a wiki source has nothing to fetch and says so.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "source": {
                            "type": "string",
                            "description": "The source's name, as list_kb_sources gave it, or its id."
                        }
                    },
                    "required": ["source"]
                }"#,
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
