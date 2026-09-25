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

/// The slug of the server that reads and writes a task's plan. Its own server rather than more
/// tools on [`MANAGE_UBIQ_TASKS`], which already carries twelve and would stop describing itself
/// with plan tools added — see `_docs/wip/planning-system.md` decision 7.
pub const UBIQ_PLAN: &str = "ubiq-plan";

/// The slug of the server a mission's **coordinator** runs the mission from (M16).
pub const UBIQ_MISSION: &str = "ubiq-mission";

/// The slug of the thinner server a mission's **workers** read it through — the same `manage` /
/// `use` split [`MANAGE_UBIQ_TASKS`] and [`USE_TASK`] already have, and for its reason: a worker
/// reads the mission and reports its own progress, it does not run it.
pub const USE_MISSION: &str = "use-mission";

/// The slug of the server that reads and writes this project's knowledge base.
pub const UBIQ_KB: &str = "ubiq-kb";

/// The slug of the server that reads Ubiq's own documentation.
pub const UBIQ_HELP: &str = "ubiq-help";

/// The slug of the server that asks the user a question and waits for the answer. The one server
/// here whose tool does not answer itself — see [`super::ask`] and `D138`.
pub const UBIQ_ASK: &str = "ubiq-ask";

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

/// The mission tools both servers answer: everything an agent in a mission **reads**, plus the
/// one line it writes about its own work. Shared for the todo tools' own reason — two tables
/// describing one tool is two descriptions that drift.
///
/// **None of them takes a mission argument.** Which mission the caller is in comes from its
/// `AgentFacts::mission`, resolved from the URL identity (M11, `D102`); an agent in no mission is
/// answered with a sentence saying so, not an error.
const MISSION_OVERVIEW: ToolSpec = ToolSpec {
    name: "mission_overview",
    description: "Everything about the mission you are in, at a glance: its phase, the brief, who is on it, how its tasks stand, which documents exist, what is waiting on the user, and the last few journal lines. Call this first.",
    schema: r#"{"type": "object", "properties": {}}"#,
};
const READ_BRIEF: ToolSpec = ToolSpec {
    name: "read_brief",
    description: "The mission's brief: the anchor task's title, key and description, its attachments, and the titles and keys of the tasks it references. What the mission is for, in the words it was written with.",
    schema: r#"{"type": "object", "properties": {}}"#,
};
const LIST_DOCUMENTS: ToolSpec = ToolSpec {
    name: "list_documents",
    description: "The names of the mission's documents. The plan is not one of them — read that with ubiq-plan's read_plan.",
    schema: r#"{"type": "object", "properties": {}}"#,
};
const READ_DOCUMENT: ToolSpec = ToolSpec {
    name: "read_document",
    description: "One mission document, whole, with the revision it stands at. A document that does not exist reads as an empty body rather than an error.",
    schema: r#"{
        "type": "object",
        "properties": {
            "name": {"type": "string", "description": "A name from list_documents, without the .md."}
        },
        "required": ["name"]
    }"#,
};
const REPORT_PROGRESS: ToolSpec = ToolSpec {
    name: "report_progress",
    description: "Write one line into the mission's journal saying what you have just done or found. The person watching reads these; write them as you go rather than in a batch at the end.",
    schema: r#"{
        "type": "object",
        "properties": {
            "text": {"type": "string", "description": "One sentence, in the past tense."},
            "task_id": {"type": "string", "description": "The task it is about, if it is about one."}
        },
        "required": ["text"]
    }"#,
};
/// What one task in a mission's breakdown says — shared by `create_mission_task` and each entry of
/// `create_mission_tasks`, so the singular and the batch can never drift apart.
const MISSION_TASK_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "title": {"type": "string", "description": "What the task is, in a line."},
        "description": {"type": "string", "description": "What doing it involves."},
        "todos": {
            "type": "array",
            "items": {"type": "string"},
            "description": "The steps, in order. Each becomes a todo on the task."
        },
        "labels": {
            "type": "array",
            "items": {"type": "string"},
            "description": "What kind of work it is, in the board's own label vocabulary. In auto mode this is what decides which agent gets the task."
        },
        "kind": {"type": "string", "description": "The agent kind this task suits, from list_agent_kinds. The scheduler spawns this kind for it."},
        "prerequisites": {
            "type": "array",
            "items": {"type": "string"},
            "description": "What this task waits on: a task id, or the key of a task that already exists. It is not ready until every one of them is in review or done."
        }
    },
    "required": ["title"]
}"#;
const LIST_AGENTS: ToolSpec = ToolSpec {
    name: "list_agents",
    description: "Who is on the mission: each member's id, role, the task it is serving and what it is doing.",
    schema: r#"{"type": "object", "properties": {}}"#,
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
                        },
                        "ready_only": {
                            "type": "boolean",
                            "description": "Only tasks whose prerequisites are all in review or done, or that have none. Omit or false for all."
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
                        "complexity": {
                            "type": "string",
                            "enum": ["low", "medium", "high"],
                            "description": "How hard the task is judged to be. Omit to leave it unsized."
                        },
                        "assigned_to": {
                            "type": "string",
                            "description": "Who has the task: a person's or an agent's name. Free text."
                        },
                        "key": {"type": "string", "description": "Human id, e.g. UBQ-123."},
                        "link": {"type": "string"},
                        "labels": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Tag names to put on the card."
                        },
                        "attachments": {
                            "type": "array",
                            "items": {
                                "oneOf": [
                                    {"type": "string"},
                                    {
                                        "type": "object",
                                        "properties": {
                                            "target": {"type": "string"},
                                            "label": {"type": "string"}
                                        },
                                        "required": ["target"]
                                    }
                                ]
                            },
                            "description": "Files and knowledge-base documents to hang on the card, as references not content. Each is a project-relative path (docs/spec.md) or a knowledge-base address (kb:{source}:{path}), optionally with a label."
                        },
                        "prerequisites": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Task ids this one waits on. It is not ready until every one of them is in review or done. A self, cross-project or cyclic prerequisite is refused."
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
                        "complexity": {
                            "type": "string",
                            "enum": ["low", "medium", "high"],
                            "description": "How hard the task is judged to be."
                        },
                        "assigned_to": {
                            "type": "string",
                            "description": "Who has the task: a person's or an agent's name. Free text."
                        },
                        "key": {"type": "string"},
                        "link": {"type": "string"},
                        "labels": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "The full tag set. Omit or null to leave tags alone; [] clears them."
                        },
                        "attachments": {
                            "type": "array",
                            "items": {
                                "oneOf": [
                                    {"type": "string"},
                                    {
                                        "type": "object",
                                        "properties": {
                                            "target": {"type": "string"},
                                            "label": {"type": "string"}
                                        },
                                        "required": ["target"]
                                    }
                                ]
                            },
                            "description": "The full attachment set, replaced — send every one the card should have. Each is a project-relative path (docs/spec.md) or a knowledge-base address (kb:{source}:{path}), optionally with a label. Omit or null to leave them alone; [] clears them."
                        },
                        "prerequisites": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "The full prerequisite set, replaced — send every task id this one should wait on. Omit or null to leave them alone; [] clears them. A self, cross-project or cyclic prerequisite is refused."
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
        // The title is what a person reads in the settings checklist; `USE_TASK` is the slug the
        // URL and the dispatch agree on, and agents are connected to it, so only the words change.
        title: "Use ubiq tasks",
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
                        },
                        "ready_only": {
                            "type": "boolean",
                            "description": "Only tasks whose prerequisites are all in review or done, or that have none. Omit or false for all."
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
        name: UBIQ_PLAN,
        title: "Plan",
        description: "Read and write the plan document for a mission — a task carrying a level — and answer the annotations left on it.",
        tools: &[
            ToolSpec {
                name: "read_plan",
                description: "A task's plan, as markdown, with the revision it stands at. Empty for a mission that has not been planned yet. Refused for a task with no level: only a task carrying a level can have a plan. Keep the revision if you intend to come back: plan_changes uses it to tell you what changed while you were away.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "task_id": {"type": "string"}
                    },
                    "required": ["task_id"]
                }"#,
            },
            ToolSpec {
                name: "write_plan",
                description: "Replace a task's plan, whole, with the markdown given. There is no partial edit: send the full document every time. Refused for a task with no level. If you read the plan first, pass expected_revision: the write is refused, and nothing is overwritten, if somebody saved in between.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "task_id": {"type": "string"},
                        "body": {"type": "string", "description": "The plan's full markdown body."},
                        "expected_revision": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "The revision you read the plan at. The write is refused if the plan has moved past it, so your edit never silently replaces somebody else's save. Omit only when you are writing a plan you did not read."
                        }
                    },
                    "required": ["task_id", "body"]
                }"#,
            },
            ToolSpec {
                name: "plan_changes",
                description: "Where the plan has been edited since you last wrote it, and by how much. Call this before rewriting a plan you wrote earlier: it returns each changed run of lines with its line numbers, the text now standing there, who changed it (human or agent) and the block it falls in, plus counts of lines added, removed and modified, blocks touched, and how many saves each side made. Defaults to your own last write_plan; pass since_revision to ask from a different point. Returns no regions when nothing has changed, which is the ordinary answer and not an error.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "task_id": {"type": "string"},
                        "since_revision": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "Report changes made after this revision. Defaults to the revision of your own last write_plan; 0 means everything that is known."
                        }
                    },
                    "required": ["task_id"]
                }"#,
            },
            ToolSpec {
                name: "list_annotations",
                description: "The plan's annotations, open ones by default, each with the text of the block it is about so you do not have to guess what the comment refers to. An annotation whose block has vanished from the plan comes back with orphaned true and block_text null: do not try to answer that one, the passage it named is gone.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "task_id": {"type": "string"},
                        "include_resolved": {
                            "type": "boolean",
                            "description": "Also return annotations already resolved. Defaults to false."
                        }
                    },
                    "required": ["task_id"]
                }"#,
            },
            ToolSpec {
                name: "reply_annotation",
                description: "Append a reply to an annotation's thread, as this agent.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "task_id": {"type": "string"},
                        "annotation_id": {"type": "string"},
                        "text": {"type": "string"}
                    },
                    "required": ["task_id", "annotation_id", "text"]
                }"#,
            },
            ToolSpec {
                name: "resolve_annotation",
                description: "Close an annotation, or reopen one. resolved defaults to true. Anyone may resolve an annotation, including the agent that answered it — there is no author check.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "task_id": {"type": "string"},
                        "annotation_id": {"type": "string"},
                        "resolved": {"type": "boolean"}
                    },
                    "required": ["task_id", "annotation_id"]
                }"#,
            },
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
    ServerSpec {
        name: UBIQ_HELP,
        title: "Ubiq's own documentation",
        description: "The manual behind the '?' in Ubiq's own titlebar: read it to answer a question about how Ubiq itself works, rather than guessing. Every page has a stable id; hand one back to the user as ubiq://./help/<id> and it opens there.",
        tools: &[
            ToolSpec {
                name: "list_help_pages",
                description: "Every page of Ubiq's own manual, in the order its table of contents draws them: id, title and summary. Call this first, or when unsure which page answers a question. If this build has no documentation, the list is empty and a note says why.",
                schema: r#"{"type": "object", "properties": {}}"#,
            },
            ToolSpec {
                name: "read_help_page",
                description: "One page's body, as Markdown with its frontmatter stripped. Accepts the page's own id or an old id/path it used to answer to.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "A page id from list_help_pages, or a redirect it used to answer to."
                        }
                    },
                    "required": ["id"]
                }"#,
            },
            ToolSpec {
                name: "search_help",
                description: "Search the manual for a word or phrase. A hit in a page's keywords, title or summary counts for more than one in its body; each result carries the matching lines.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "A word or phrase to search for."}
                    },
                    "required": ["query"]
                }"#,
            },
            ToolSpec {
                name: "help_for_context",
                description: "The page bound to a place in the interface, the same lookup the '?' in the titlebar does for the screen currently open — pass a panel name, a view or a rail mode's key.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "key": {
                            "type": "string",
                            "description": "A context key, e.g. 'panel.ubiq.git-changes', 'rail.git', 'view.chat', or 'index' for the landing page."
                        }
                    },
                    "required": ["key"]
                }"#,
            },
        ],
    },
    ServerSpec {
        name: UBIQ_ASK,
        title: "Ask the user",
        description: "Ask the person watching a structured question and wait for their answer. Use it when a choice is theirs to make — an approach, a trade-off, a name — rather than guessing and writing something they did not ask for. The call blocks until they answer, so ask once, ask everything you need at once, and keep working from the answer.",
        tools: &[ToolSpec {
            name: "ask_user_question",
            description: "Put one to four multiple-choice questions to the user and wait for their reply. Each question shows two to four options you wrote; 'Other' is always offered beside them and is never one of yours, so do not write it. The answer names the options the user picked by their labels, plus anything they typed. If the user would rather talk it through, the result says so and they will say the rest in the chat — carry on from the conversation, do not ask again.",
            schema: r#"{
                "type": "object",
                "properties": {
                    "questions": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 4,
                        "description": "The questions to ask, at most four. Ask everything you need in one call.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "question": {
                                    "type": "string",
                                    "description": "The question in full, ending with a question mark."
                                },
                                "header": {
                                    "type": "string",
                                    "description": "A label of at most 12 characters, drawn as the question's tab. E.g. 'Database'."
                                },
                                "options": {
                                    "type": "array",
                                    "minItems": 2,
                                    "maxItems": 4,
                                    "description": "Two to four options. Do not offer 'Other' — it is always there.",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "label": {
                                                "type": "string",
                                                "description": "What the control says: a few words."
                                            },
                                            "description": {
                                                "type": "string",
                                                "description": "What picking it means, drawn under the label."
                                            },
                                            "preview": {
                                                "type": "string",
                                                "description": "Something to show beside the choice — a snippet, a sketch, a diff. Single-select questions only."
                                            }
                                        },
                                        "required": ["label"]
                                    }
                                },
                                "multiSelect": {
                                    "type": "boolean",
                                    "description": "Whether more than one option may be picked. A multi-select question may carry no previews."
                                }
                            },
                            "required": ["question", "header", "options"]
                        }
                    }
                },
                "required": ["questions"]
            }"#,
        }],
    },
    ServerSpec {
        name: UBIQ_MISSION,
        title: "Run the mission",
        description: "The mission you are coordinating: its phase, brief, roster, tasks, documents and journal. Use it to read where the mission stands, write its documents, create its tasks, keep the person watching informed, and ask for the phase to move when a stage is done. The mission is resolved from who you are — no call here takes a mission id.",
        tools: &[
            MISSION_OVERVIEW,
            READ_BRIEF,
            LIST_DOCUMENTS,
            READ_DOCUMENT,
            ToolSpec {
                name: "write_document",
                description: "Create or replace one of the mission's documents. Pass expected_revision with the revision read_document gave you, and the write is refused if anything changed underneath — read it again, redo your edit, and write with the revision the refusal names. Omit it only for a document nobody has written yet.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "A bare name, no path and no .md — e.g. 'architecture'."},
                        "body": {"type": "string", "description": "The whole document, as Markdown. This replaces what is there."},
                        "expected_revision": {"type": "integer", "minimum": 0, "description": "The revision you read. Omit for a document that does not exist yet."}
                    },
                    "required": ["name", "body"]
                }"#,
            },
            ToolSpec {
                name: "create_mission_task",
                description: "Create one task inside the mission, as a child of its anchor. Give it todos if the work splits into steps. Returns the task's id and the key a person says out loud. Writing a whole breakdown? Use create_mission_tasks instead — one call, and a task can wait on another in the same call.",
                schema: MISSION_TASK_SCHEMA,
            },
            ToolSpec {
                name: "create_mission_tasks",
                description: "Write the mission's whole work breakdown in one call. Each task may wait on tasks created earlier in the same call: give an entry a short 'ref' and name that ref in a later entry's 'prerequisites'. Label every task for affinity (area, component, skill) — in auto mode the scheduler hands a task to the agent whose past tasks share its labels, so the labels are what decides who works on what. Keep each task small enough for one agent. Tasks are created in the order given; the answer names what was created and anything that failed, so a second call can fill the gaps.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "tasks": {
                            "type": "array",
                            "description": "The breakdown, in order. An entry may name an earlier entry's ref as a prerequisite.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "title": {"type": "string", "description": "What the task is, in a line."},
                                    "description": {"type": "string", "description": "What doing it involves."},
                                    "ref": {"type": "string", "description": "A short name for this entry, used only inside this call so later entries can wait on it. It is not written down anywhere."},
                                    "todos": {"type": "array", "items": {"type": "string"}, "description": "The steps, in order. Each becomes a todo on the task."},
                                    "labels": {"type": "array", "items": {"type": "string"}, "description": "What kind of work it is, in the board's own label vocabulary. This is what affinity is computed from."},
                                    "kind": {"type": "string", "description": "The agent kind this task suits, from list_agent_kinds. The scheduler spawns this kind for it."},
                                    "prerequisites": {"type": "array", "items": {"type": "string"}, "description": "What this task waits on: a task id, the key of a task that already exists, or the ref of an entry earlier in this same call. It is not ready until every one of them is in review or done."}
                                },
                                "required": ["title"]
                            }
                        }
                    },
                    "required": ["tasks"]
                }"#,
            },
            REPORT_PROGRESS,
            ToolSpec {
                name: "request_phase",
                description: "Ask for the mission to move to another phase, with a summary of why. Some moves happen at once and some wait for the user to confirm; the answer says which happened. Do not ask twice — a second request replaces the first.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "phase": {
                            "type": "string",
                            "enum": ["requirements", "refining", "in progress", "completed", "abandoned"],
                            "description": "The phase to move to."
                        },
                        "summary": {"type": "string", "description": "Why, in a sentence or two. The user reads this before answering."}
                    },
                    "required": ["phase"]
                }"#,
            },
            ToolSpec {
                name: "list_agent_kinds",
                description: "The kinds of agent this mission can spawn, each with a description of what it is for. Read this before spawn_agent and ask for one by name. An empty list means nobody has set the mission's agent kinds up yet — say what you need and why, and a person will.",
                schema: r#"{"type": "object", "properties": {}}"#,
            },
            ToolSpec {
                name: "spawn_agent",
                description: "Ask the mission for another agent, of one of the kinds list_agent_kinds names. This returns a request id straight away and does NOT wait: the person watching decides whether it is launched, and may change the kind first. You will be told the outcome, including which kind was actually used, as a later message — carry on with something else until then. Do not ask twice for the same work.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "kind": {"type": "string", "description": "A kind from list_agent_kinds, by name. Omit for the mission's default kind. Use 'custom' only with a profile."},
                        "profile": {"type": "string", "description": "A saved profile, for kind 'custom' only. Never a harness, an account or anything secret."},
                        "task_id": {"type": "string", "description": "The task this agent is for, if there is one."},
                        "prompt": {"type": "string", "description": "What to tell it when it starts — the whole of what it needs to begin."},
                        "reason": {"type": "string", "description": "Why you need it, in a sentence. The person reads this before answering."}
                    },
                    "required": ["prompt", "reason"]
                }"#,
            },
            LIST_AGENTS,
            ToolSpec {
                name: "message_agent",
                description: "Put a line in another mission member's thread. It reads it as its next prompt, or when it finishes what it is doing. Only agents on the mission's roster can be messaged.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "agent_id": {"type": "string", "description": "An agent id from list_agents."},
                        "text": {"type": "string", "description": "What to say to it."}
                    },
                    "required": ["agent_id", "text"]
                }"#,
            },
            ToolSpec {
                name: "read_feedback",
                description: "What the user has said to the mission since you last asked. Empty when there is nothing new. Read it between pieces of work — it is how the person watching steers you.",
                schema: r#"{"type": "object", "properties": {}}"#,
            },
        ],
    },
    ServerSpec {
        name: USE_MISSION,
        title: "Use the mission",
        description: "The mission you are working in: what it is for, where it has got to, who else is on it, and what has been written down. Read it before you start, and report your progress as you go. You do not run the mission — its coordinator does.",
        tools: &[
            MISSION_OVERVIEW,
            READ_BRIEF,
            LIST_DOCUMENTS,
            READ_DOCUMENT,
            REPORT_PROGRESS,
            LIST_AGENTS,
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
