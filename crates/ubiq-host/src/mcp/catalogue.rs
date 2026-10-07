//! What Ubiq offers an agent: the built-in servers, their tools, and the argument schemas those
//! tools take.
//!
//! **One table, three readers.** The settings panel's checklist comes from here through
//! [`Message::Mcps`], the `tools/list` a harness asks for comes from here, and
//! [`crate::agent::Agents::compose_run`] asks here whether a name a definition saved is one this
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

/// The slug of the server that collaborates on **any** markdown file in the project: the same
/// annotation handlers `ubiq-plan` runs, with the document named by a project-relative path
/// instead of a task (`D161`'s `DocumentHandle::File`). Not in a default set — a definition names
/// it.
pub const UBIQ_DOC: &str = "ubiq-doc";

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

/// The slug of the server that asks the user a question. The one server here whose tools do not
/// answer from a fact the host holds: `ask_user_question` parks until a person answers (`D138`),
/// `register_question` arms a dialog for the end of the turn (`D175`) — see [`super::ask`].
pub const UBIQ_ASK: &str = "ubiq-ask";

/// The slug of the diagram server: validate, lay out, render and create Archify diagrams under the
/// agent's own project folder. Not in a default set — a definition names it.
pub const UBIQ_ARCHIFY: &str = "ubiq-archify";

/// The slug of the SQL server that only reads: every connection the user let agents use, run under
/// the engine's read-only guard (`D202`). Not in a default set — a definition names it.
pub const UBIQ_SQL_READ: &str = "ubiq-sql-read";

/// The slug of the SQL server that also writes: the read tools plus `execute`, on connections the
/// user marked read-write only (`D202`). Not in a default set — a definition names it.
pub const UBIQ_SQL_WRITE: &str = "ubiq-sql-write";

/// What a **mission/task coordinator** agent definition runs with: it runs the mission, writes
/// the plan, manages the tasks, and keeps the knowledge base. The `manage` side of every split
/// pair, because running the work is what a coordinator does.
///
/// Held here rather than beside the definitions, so the set and the slugs it names cannot drift.
pub const COORDINATOR_MCPS: &[&str] = &[UBIQ_MISSION, UBIQ_PLAN, MANAGE_UBIQ_TASKS, UBIQ_KB];

/// What a **mission/task worker** agent definition runs with: the `use` side of the mission and
/// task servers, what project it is working in, and the knowledge base.
pub const WORKER_MCPS: &[&str] = &[USE_MISSION, USE_TASK, PROJECT_INFO, UBIQ_KB];

/// What a **document** agent definition runs with: the annotated-document server and the
/// architecture-diagram one, for working on markdown together with the user.
pub const DOC_MCPS: &[&str] = &[UBIQ_DOC, UBIQ_ARCHIFY];

/// The servers the three role tags (`coordinator`, `worker`, `doc`) imply between them, in
/// catalogue order and without repeats — a definition carrying several roles gets every set, and
/// `ubiq-kb` only once.
pub fn role_mcps(coordinator: bool, worker: bool, doc: bool) -> Vec<String> {
    let mut named: Vec<String> = Vec::new();
    let sets = [
        (coordinator, COORDINATOR_MCPS),
        (worker, WORKER_MCPS),
        (doc, DOC_MCPS),
    ];
    for (on, set) in sets {
        if !on {
            continue;
        }
        for name in set {
            if !named.iter().any(|it| it == name) {
                named.push((*name).to_string());
            }
        }
    }
    named
}

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
/// The `questions` array both `ubiq-ask` tools take: the two modes differ in when the dialog is
/// raised and where the answer is delivered, never in what may be asked.
const ASK_QUESTIONS_SCHEMA: &str = r#"{
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
            }"#;

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
    description: "The mission's brief: the anchor task's title, key and description, the documents attached to it, and the titles and keys of the tasks it references. What the mission is for, in the words it was written with. Attach another document with attach_document.",
    schema: r#"{"type": "object", "properties": {}}"#,
};
const LIST_DOCUMENTS: ToolSpec = ToolSpec {
    name: "list_documents",
    description: "The names of the mission's documents. The plan is not one of them — read that with ubiq-plan's read_plan.",
    schema: r#"{"type": "object", "properties": {}}"#,
};
/// The reference one attachment carries — the same two forms a task attachment takes, because an
/// attached document *is* a task attachment on the mission's anchor (M7). Nothing resolves it here.
const ATTACH_TARGET_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "target": {"type": "string", "description": "A project-relative path such as 'docs/spec.md', or a knowledge-base address 'kb:{source}:{path}' as list_kb_documents gives it."},
        "label": {"type": "string", "description": "What to call it. Optional — without one the file's own name is shown."}
    },
    "required": ["target"]
}"#;
const ATTACH_DOCUMENT: ToolSpec = ToolSpec {
    name: "attach_document",
    description: "Attach an existing document to the mission's brief, so everyone on the mission can find it: a file in the project, or a knowledge-base entry. This does not write or copy anything — it stores the reference, and read_brief lists it. Not the same as write_document, which creates a document of the mission's own. Attaching something already attached only updates its label.",
    schema: ATTACH_TARGET_SCHEMA,
};
const DETACH_DOCUMENT: ToolSpec = ToolSpec {
    name: "detach_document",
    description: "Remove a document attachment from the mission's brief, by the exact target attach_document was given. The document itself is untouched.",
    schema: r#"{
        "type": "object",
        "properties": {
            "target": {"type": "string", "description": "The target exactly as read_brief gives it."}
        },
        "required": ["target"]
    }"#,
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

// ── the SQL servers ───────────────────────────────────────────────────────────────────────────
//
// Both servers answer the same tools bar `execute`, which only the write server has. The
// parameter names are the contract: `max_rows` and `max_field_size` are required on purpose — an
// agent that has to choose a size has to think about what it is about to read into its context.

const SQL_LIST_CONNECTIONS: ToolSpec = ToolSpec {
    name: "list_connections",
    description: "The database connections you may use: name, engine, database, access (ro or rw), description and which is the default. Omit `connection` on the other tools to use the default.",
    schema: r#"{"type": "object", "properties": {}}"#,
};
const SQL_LIST_OBJECTS: ToolSpec = ToolSpec {
    name: "list_objects",
    description: "List what a connection holds: with no `database` its databases, with one its schemas, with a `schema` its tables and views. Cheap; use it before writing SQL against names you have guessed.",
    schema: r#"{
        "type": "object",
        "properties": {
            "connection": {"type": "string", "description": "Connection name or id. Omit for the default."},
            "database": {"type": "string"},
            "schema": {"type": "string"}
        }
    }"#,
};
const SQL_DESCRIBE_TABLE: ToolSpec = ToolSpec {
    name: "describe_table",
    description: "A table's columns: type, nullability, key, default. Use it instead of `select *` to learn a table's shape.",
    schema: r#"{
        "type": "object",
        "properties": {
            "connection": {"type": "string", "description": "Connection name or id. Omit for the default."},
            "database": {"type": "string"},
            "schema": {"type": "string"},
            "table": {"type": "string"}
        },
        "required": ["table"]
    }"#,
};
const SQL_EXPORT_DBML: ToolSpec = ToolSpec {
    name: "export_dbml",
    description: "A database's structure as DBML text, reverse-engineered from the catalog: tables with their comments; columns with type, primary key, increment, not null, unique, default and comment; indexes, composite and unique ones included; foreign keys as Refs with their delete and update actions; enums (Postgres enum types, MySQL enum columns). Views, sequences, triggers and check constraints are left out, and a Ref is written only when both its tables are exported. Tables in the default schema (public, dbo) are written without it. Narrow it with `schema` or `tables` on a large database.",
    schema: r#"{
        "type": "object",
        "properties": {
            "connection": {"type": "string", "description": "Connection name or id. Omit for the default."},
            "database": {"type": "string", "description": "Omit for the connection's own."},
            "schema": {"type": "string", "description": "Only this schema. Omit for every schema bar the system ones."},
            "tables": {"type": "array", "items": {"type": "string"}, "description": "Only these tables, each `name` or `schema.name`."},
            "timeout_ms": {"type": "integer", "minimum": 100, "maximum": 300000, "default": 30000}
        }
    }"#,
};
const SQL_QUERY: ToolSpec = ToolSpec {
    name: "query",
    description: "Run one read-only SQL statement. Results come back as TOON, a table with the column names once. At most `max_rows` rows are read (`more_rows` says there were more). A text cell longer than `max_field_size` characters is cut and ends in `[blob:<id>:<total>]`; binary cells are always `[blob:...]`. Start with a small `max_field_size` (100 to 200) and call get_blob for the one cell you need whole. Pass `panel` to also show the query and its result to the user in a named query panel.",
    schema: r#"{
        "type": "object",
        "properties": {
            "sql": {"type": "string"},
            "connection": {"type": "string", "description": "Connection name or id. Omit for the default."},
            "database": {"type": "string"},
            "max_rows": {"type": "integer", "minimum": 1, "maximum": 10000},
            "max_field_size": {"type": "integer", "minimum": 0, "maximum": 100000, "description": "Characters kept per text cell."},
            "timeout_ms": {"type": "integer", "minimum": 100, "maximum": 300000, "default": 30000},
            "panel": {"type": "string", "maxLength": 64, "description": "Show the query and result to the user in the query panel of this name."}
        },
        "required": ["sql", "max_rows", "max_field_size"]
    }"#,
};
const SQL_EXECUTE: ToolSpec = ToolSpec {
    name: "execute",
    description: "Run up to 20 statements that may change data or schema, one after another, each committed on its own. Stops at the first failure and reports what ran. Only on connections with read-write access. Rows a statement returns (`returning`) obey `max_rows` and `max_field_size` as in query. Pass `panel` to also show the statements to the user in a named query panel.",
    schema: r#"{
        "type": "object",
        "properties": {
            "statements": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 20},
            "connection": {"type": "string", "description": "Connection name or id. Omit for the default."},
            "database": {"type": "string"},
            "max_rows": {"type": "integer", "minimum": 1, "maximum": 10000},
            "max_field_size": {"type": "integer", "minimum": 0, "maximum": 100000, "description": "Characters kept per text cell."},
            "timeout_ms": {"type": "integer", "minimum": 100, "maximum": 300000, "default": 30000},
            "panel": {"type": "string", "maxLength": 64, "description": "Show the statements and results to the user in the query panel of this name."}
        },
        "required": ["statements", "max_rows", "max_field_size"]
    }"#,
};
const SQL_GET_BLOB: ToolSpec = ToolSpec {
    name: "get_blob",
    description: "Read more of a cell that a result cut or blobbed, by the id in its `[blob:<id>:<len>]` marker. `offset` and `length` count characters for text and bytes for binary, which comes back base64. Blobs expire a few minutes after you last read them; re-run the query if one is gone.",
    schema: r#"{
        "type": "object",
        "properties": {
            "id": {"type": "string"},
            "offset": {"type": "integer", "minimum": 0, "default": 0},
            "length": {"type": "integer", "minimum": 1, "maximum": 100000, "default": 20000}
        },
        "required": ["id"]
    }"#,
};
const SQL_OPEN_EDITOR: ToolSpec = ToolSpec {
    name: "open_editor",
    description: "Open (or find) a named SQL editor shared with the user, marked in their window as controlled by you. Give it `text` to put a query in it for the user to review or run. The user can edit it too: read_editor shows their changes.",
    schema: r#"{
        "type": "object",
        "properties": {
            "name": {"type": "string", "maxLength": 64},
            "connection": {"type": "string", "description": "Connection name or id. Omit for the default."},
            "database": {"type": "string"},
            "text": {"type": "string"}
        },
        "required": ["name"]
    }"#,
};
const SQL_READ_EDITOR: ToolSpec = ToolSpec {
    name: "read_editor",
    description: "The current text of a named editor, with its revision and whether anyone but you changed it since your last write.",
    schema: r#"{
        "type": "object",
        "properties": {"name": {"type": "string", "maxLength": 64}},
        "required": ["name"]
    }"#,
};
const SQL_EDIT_EDITOR: ToolSpec = ToolSpec {
    name: "edit_editor",
    description: "Replace a named editor's text. `mode` decides what happens if the user changed it since your last write: `force` overwrites regardless; `overwrite_return_previous` overwrites and hands back their text if they had changed it; `keep_if_user_changed` writes nothing if they had and returns the current text.",
    schema: r#"{
        "type": "object",
        "properties": {
            "name": {"type": "string", "maxLength": 64},
            "text": {"type": "string"},
            "mode": {"type": "string", "enum": ["force", "overwrite_return_previous", "keep_if_user_changed"]}
        },
        "required": ["name", "text", "mode"]
    }"#,
};
const SQL_RUN_EDITOR: ToolSpec = ToolSpec {
    name: "run_editor",
    description: "Run the text of a named editor (every statement in it, at most 20) on its connection and show the result in that editor's panel. Read-only on the read server; the write server runs it with write access. Output and limits as in query.",
    schema: r#"{
        "type": "object",
        "properties": {
            "name": {"type": "string", "maxLength": 64},
            "max_rows": {"type": "integer", "minimum": 1, "maximum": 10000},
            "max_field_size": {"type": "integer", "minimum": 0, "maximum": 100000, "description": "Characters kept per text cell."},
            "timeout_ms": {"type": "integer", "minimum": 100, "maximum": 300000, "default": 30000}
        },
        "required": ["name", "max_rows", "max_field_size"]
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
                        "level": {
                            "type": "string",
                            "enum": ["mission"],
                            "description": "Promote the task to a mission — allowed to have children and to carry a plan. Omit for an ordinary task."
                        },
                        "shape": {
                            "type": "string",
                            "enum": ["direct", "chain", "coordinated"],
                            "description": "How the agents on it are arranged: one agent asked directly, a hand-off chain, or a coordinator splitting work across workers. Omit if nobody has said."
                        },
                        "parent": {
                            "type": "string",
                            "description": "The mission task this one is a child of, by id. Only a task with level mission may be a parent."
                        },
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
                        "references": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Other task ids this one is related to — untyped and symmetric, unlike prerequisites. Shows as a chip list linking to the other card."
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
                        "level": {
                            "type": "string",
                            "enum": ["mission"],
                            "description": "Promote the task to a mission — allowed to have children and to carry a plan. There is no way to demote it back through this tool."
                        },
                        "shape": {
                            "type": "string",
                            "enum": ["direct", "chain", "coordinated"],
                            "description": "How the agents on it are arranged: one agent asked directly, a hand-off chain, or a coordinator splitting work across workers."
                        },
                        "parent": {
                            "type": "string",
                            "description": "The mission task this one is a child of, by id. Only a task with level mission may be a parent. Omit or null to leave it alone; an empty string clears it."
                        },
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
                        "references": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "The full reference set, replaced — untyped and symmetric, unlike prerequisites. Omit or null to leave them alone; [] clears them."
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
                        "task_id": {"type": "string", "description": "The task's id, or its key (e.g. T-166) if it has one."}
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
                        "task_id": {"type": "string", "description": "The task's id, or its key (e.g. T-166) if it has one."}
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
                description: "Replace a task's plan, whole, with the markdown given. There is no partial edit: send the full document every time. Refused for a task with no level. Read the plan with read_plan first: if the user edits it while you work, your write is merged with theirs instead of replacing it — where you both changed the same words the user's text is kept and your version is posted as a thread on that block (the result then carries conflicts.threads). Always continue from the body the result returns.",
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
                description: "The plan's annotations, open ones by default, each with the text of the block it is about so you do not have to guess what the comment refers to. An annotation whose block has vanished from the plan comes back with orphaned true and block_text null: do not try to answer that one, the passage it named is gone. review true means you proposed it resolved and the user has not ruled yet: leave it be.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "task_id": {"type": "string"},
                        "include_resolved": {
                            "type": "boolean",
                            "description": "Also return annotations already resolved. Defaults to false."
                        },
                        "mark": {
                            "type": "string",
                            "enum": ["agent", "todo", "question"],
                            "description": "Only annotations carrying this mark."
                        }
                    },
                    "required": ["task_id"]
                }"#,
            },
            ToolSpec {
                name: "annotate_plan",
                description: "Open a new annotation thread on the plan, as this agent. Say where with quote (a passage that appears in exactly one block; refused if none or several) or block_id (from list_annotations or read), which wins over quote.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "task_id": {"type": "string"},
                        "text": {"type": "string"},
                        "quote": {"type": "string"},
                        "block_id": {"type": "string"},
                        "marks": {
                            "type": "array",
                            "items": {"type": "string", "enum": ["agent", "todo", "question"]}
                        }
                    },
                    "required": ["task_id", "text"]
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
                description: "Propose an annotation resolved, or reopen one. You cannot close a thread: resolved (default true) marks it awaiting the user's review — it stays open with the review mark, and only the user's Accept resolves it (their Reopen sends it back to you). Pass note to say what you did; it is added to the thread as your comment. resolved false reopens it.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "task_id": {"type": "string"},
                        "annotation_id": {"type": "string"},
                        "resolved": {"type": "boolean"},
                        "note": {"type": "string", "description": "What you did about it — added to the thread as your comment."}
                    },
                    "required": ["task_id", "annotation_id"]
                }"#,
            },
        ],
    },
    ServerSpec {
        name: UBIQ_DOC,
        title: "Annotated documents",
        description: "Work on any markdown file in this project together with the user: read and write it, and answer the annotations the user left on it in annotation mode.",
        tools: &[
            ToolSpec {
                name: "list_annotated_docs",
                description: "The markdown files in this project that carry annotations, with how many threads are open on each. Call this first when you have not been told which document to look at. Files with nothing open are left out unless include_resolved is true.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "include_resolved": {
                            "type": "boolean",
                            "description": "Also list files whose annotations are all resolved. Defaults to false."
                        }
                    }
                }"#,
            },
            ToolSpec {
                name: "list_my_docs",
                description: "Your working documents: the markdown files of this project the user has bound to you, with how many threads are open on each and whether auto_send is on (every comment the user leaves there is sent to you). A file bound to another agent cannot be written, annotated, replied on or resolved by you: ask the user to reassign it.",
                schema: r#"{
                    "type": "object",
                    "properties": {}
                }"#,
            },
            ToolSpec {
                name: "read_doc",
                description: "A markdown file of this project, whole, with the revision it stands at. Empty for a file that does not exist yet. Keep the revision: write_doc and doc_changes use it. An annotated file also lists its blocks, each with a lineage code (AAA, AAB…; a block split in two becomes AAB.AA and AAB.AB) that says where a block came from — informative only: anchor threads by block id or quote.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Project-relative path of a .md file."}
                    },
                    "required": ["path"]
                }"#,
            },
            ToolSpec {
                name: "write_doc",
                description: "Replace a markdown file of this project, whole, with the markdown given. Edit annotated files through this tool rather than your own file editing: it re-anchors the annotations to the new text, and a thread whose passage is gone is marked orphaned rather than lost. Read the file with read_doc first: if the user edits it while you work, your write is merged with theirs instead of replacing it — edits to different lines or words both land, and where you both changed the same words the user's text is kept and your version is posted as a thread on that block (the result then carries conflicts.threads). Always continue from the body the result returns, not from what you sent.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Project-relative path of a .md file."},
                        "body": {"type": "string", "description": "The file's full markdown body."},
                        "expected_revision": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "The revision you read the file at. Omit only when you are writing a file you did not read."
                        }
                    },
                    "required": ["path", "body"]
                }"#,
            },
            ToolSpec {
                name: "doc_changes",
                description: "Where the file has been edited since you last wrote it with write_doc, and by whom (human or agent): each changed run of lines with its line numbers and text, the block it falls in, and counts. Pass since_revision to ask from a different point. Returns no regions when nothing has changed.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "since_revision": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "Report changes made after this revision. Defaults to the revision of your own last write_doc; 0 means everything that is known."
                        }
                    },
                    "required": ["path"]
                }"#,
            },
            ToolSpec {
                name: "list_annotations",
                description: "The file's annotations, open ones by default, each with the text of the block it is about. An annotation whose block has vanished comes back with orphaned true and block_text null: do not try to answer that one. review true means you proposed it resolved and the user has not ruled yet: leave it be.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "include_resolved": {
                            "type": "boolean",
                            "description": "Also return annotations already resolved. Defaults to false."
                        },
                        "mark": {
                            "type": "string",
                            "enum": ["agent", "todo", "question"],
                            "description": "Only annotations carrying this mark."
                        }
                    },
                    "required": ["path"]
                }"#,
            },
            ToolSpec {
                name: "annotate_doc",
                description: "Open a new annotation thread on the file, as this agent. Say where with quote (a passage that appears in exactly one block; refused if none or several) or block_id (from list_annotations), which wins over quote.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "text": {"type": "string"},
                        "quote": {"type": "string"},
                        "block_id": {"type": "string"},
                        "marks": {
                            "type": "array",
                            "items": {"type": "string", "enum": ["agent", "todo", "question"]}
                        }
                    },
                    "required": ["path", "text"]
                }"#,
            },
            ToolSpec {
                name: "reply_annotation",
                description: "Append a reply to an annotation's thread, as this agent.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "annotation_id": {"type": "string"},
                        "text": {"type": "string"}
                    },
                    "required": ["path", "annotation_id", "text"]
                }"#,
            },
            ToolSpec {
                name: "resolve_annotation",
                description: "Propose an annotation resolved, or reopen one. You cannot close a thread: resolved (default true) marks it awaiting the user's review — it stays open with the review mark, and only the user's Accept resolves it (their Reopen sends it back to you). Pass note to say what you did; it is added to the thread as your comment. resolved false reopens it.",
                schema: r#"{
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "annotation_id": {"type": "string"},
                        "resolved": {"type": "boolean"},
                        "note": {"type": "string", "description": "What you did about it — added to the thread as your comment."}
                    },
                    "required": ["path", "annotation_id"]
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
        name: UBIQ_ARCHIFY,
        title: "Archify diagrams",
        description: "Validate, lay out, render and create Archify diagrams (.archify files) in the project: architecture, workflow, sequence, dataflow and lifecycle. Call archify_guide first for the authoring rules; write the file with your own tools, then archify_validate it and archify_render it so the user sees it.",
        tools: &[
            ToolSpec {
                name: "archify_schema",
                description: "The JSON Schema of a diagram type, verbatim. Call it before using a field or enum you have not used. `common` holds the shared definitions the others $ref.",
                schema: r#"{
                  "type": "object",
                  "properties": {
                    "type": {
                      "type": "string",
                      "enum": [
                        "architecture",
                        "workflow",
                        "sequence",
                        "dataflow",
                        "lifecycle",
                        "common"
                      ]
                    }
                  },
                  "required": [
                    "type"
                  ],
                  "additionalProperties": false
                }"#,
            },
            ToolSpec {
                name: "archify_validate",
                description: "Validate a diagram document: JSON syntax, meta.output path, JSON Schema, then the graph checks (relationship ids, engineering profile, evidence shape, references, grid cells, ranges, workflow contract) and, for architecture, dataflow, sequence and lifecycle v2, the geometry and composition gates. Give a project `path` or an inline `document`. Returns a receipt {ok, stage?, error?, diagnostics[], stagesRun, notChecked}. ok:false is never success: repair from diagnostics[] and validate again.",
                schema: r#"{
                  "type": "object",
                  "properties": {
                    "path": {
                      "type": "string",
                      "description": "A .archify (or <name>.<type>.json) file inside the project: relative to the project root, or absolute."
                    },
                    "document": {
                      "description": "The diagram inline: a JSON object, or a string holding JSON. `json` is accepted as an alias."
                    },
                    "json": {
                      "description": "Alias of `document`."
                    },
                    "type": {
                      "type": "string",
                      "enum": [
                        "architecture",
                        "workflow",
                        "sequence",
                        "dataflow",
                        "lifecycle"
                      ],
                      "description": "Defaults to the document's diagram_type (a .archify file has no other source), then the file name's <type>.json suffix."
                    },
                    "quality": {
                      "type": "string",
                      "enum": [
                        "advisory",
                        "standard",
                        "showcase"
                      ],
                      "description": "Overrides meta.quality_profile for gate severity. Never changes the layout."
                    },
                    "repo_root": {
                      "type": "string",
                      "description": "The Git checkout the document's source evidence (meta.repository and nodes' sources) is verified against, committed blobs at the pinned SHA: a directory inside the project, relative to it or absolute. Default: the root of your project. It must be the repository's top-level directory."
                    }
                  },
                  "additionalProperties": false
                }"#,
            },
            ToolSpec {
                name: "archify_layout",
                description: "The resolved layout of a document, as repair evidence: viewBox, node rects, edge route points and label rects. Architecture is Archify's --layout-json report (components, boundaries, connections with exact points and labelAt, labels); a layout the gates reject is returned too, with ok:false, error and diagnostics[] (contract archify-architecture-layout-v1). Dataflow, sequence and lifecycle return a receipt of the same shape (contract ubiq-archify-layout-v1: nodes|participants|states, flows|messages|transitions with points and label, frames|segments|bands); there a document the gates reject is the ordinary validate receipt. Workflow is the compiler's receipt (contract fixed-v1 or readable-v2: viewBox, requiredViewBox, columns, nodes, edges with points, labels, diagnostics); a layout the gates reject returns {contract, diagnostics[]}. Use it once after two focused repairs, not before the first validate. Give a project `path` or an inline `document`.",
                schema: r#"{
                  "type": "object",
                  "properties": {
                    "path": {
                      "type": "string",
                      "description": "A .archify (or <name>.<type>.json) file inside the project: relative to the project root, or absolute."
                    },
                    "document": {
                      "description": "The diagram inline: a JSON object, or a string holding JSON. `json` is accepted as an alias."
                    },
                    "json": {
                      "description": "Alias of `document`."
                    },
                    "type": {
                      "type": "string",
                      "enum": [
                        "architecture",
                        "workflow",
                        "sequence",
                        "dataflow",
                        "lifecycle"
                      ],
                      "description": "Defaults to the document's diagram_type (a .archify file has no other source), then the file name's <type>.json suffix."
                    },
                    "quality": {
                      "type": "string",
                      "enum": [
                        "advisory",
                        "standard",
                        "showcase"
                      ],
                      "description": "Overrides meta.quality_profile for gate severity of a rejected layout. Never changes the layout."
                    },
                    "repo_root": {
                      "type": "string",
                      "description": "The Git checkout the document's source evidence (meta.repository and nodes' sources) is verified against, committed blobs at the pinned SHA: a directory inside the project, relative to it or absolute. Default: the root of your project. It must be the repository's top-level directory."
                    }
                  },
                  "additionalProperties": false
                }"#,
            },
            ToolSpec {
                name: "archify_render",
                description: "Compile a diagram and show it in Ubiq: the diagram viewer opens or focuses the file's tab. Returns {ok, type, profile, counts{nodes,edges,frames}, viewBox, diagnostics[], shown}; no HTML. A document that fails to compile is the ordinary validate receipt (ok:false) and nothing is shown. Call it once the document validates. Give a project `path` (shown) or an inline `document` (summarised only).",
                schema: r#"{
                  "type": "object",
                  "properties": {
                    "path": {
                      "type": "string",
                      "description": "A .archify (or <name>.<type>.json) file inside the project: relative to the project root, or absolute."
                    },
                    "document": {
                      "description": "The diagram inline: a JSON object, or a string holding JSON. `json` is accepted as an alias."
                    },
                    "json": {
                      "description": "Alias of `document`."
                    },
                    "type": {
                      "type": "string",
                      "enum": [
                        "architecture",
                        "workflow",
                        "sequence",
                        "dataflow",
                        "lifecycle"
                      ],
                      "description": "Defaults to the document's diagram_type (a .archify file has no other source), then the file name's <type>.json suffix."
                    },
                    "quality": {
                      "type": "string",
                      "enum": [
                        "advisory",
                        "standard",
                        "showcase"
                      ],
                      "description": "Overrides meta.quality_profile for gate severity. Never changes the layout."
                    },
                    "repo_root": {
                      "type": "string",
                      "description": "The Git checkout the document's source evidence (meta.repository and nodes' sources) is verified against, committed blobs at the pinned SHA: a directory inside the project, relative to it or absolute. Default: the root of your project. It must be the repository's top-level directory."
                    }
                  },
                  "additionalProperties": false
                }"#,
            },
            ToolSpec {
                name: "archify_new",
                description: "Create a new diagram file: a minimal valid starter of `type`, titled `name`, written to <dir>/<slug>.archify (the slug comes from `name`; `dir` is relative to the project root, default the root itself, created if missing). Refuses to overwrite an existing file. Opens its tab in Ubiq. Returns {ok, path, type}; then edit the file and validate it. An absolute `dir` must be inside the project.",
                schema: r#"{
                  "type": "object",
                  "properties": {
                    "type": {
                      "type": "string",
                      "enum": [
                        "architecture",
                        "workflow",
                        "sequence",
                        "dataflow",
                        "lifecycle"
                      ]
                    },
                    "name": {
                      "type": "string",
                      "description": "The diagram's title; the file is named after it."
                    },
                    "dir": {
                      "type": "string",
                      "description": "Folder inside the project, e.g. \"diagrams\". Default: the project root."
                    }
                  },
                  "required": [
                    "type",
                    "name"
                  ],
                  "additionalProperties": false
                }"#,
            },
            ToolSpec {
                name: "archify_guide",
                description: "The authoring guide. Without arguments it returns the main skill; with a `topic`, that reference (an unknown topic lists the valid ones). With a `scenario` (plain words: what the diagram should explain) it scores 12 recipes by keyword and returns the best one and up to two alternatives, each with the diagram type to use, what it must include and starter prompts. Not both.",
                schema: r#"{
                  "type": "object",
                  "properties": {
                    "topic": {
                      "type": "string",
                      "enum": [
                        "architecture-layout-repair",
                        "architecture",
                        "authoring-defaults",
                        "dataflow",
                        "evidence",
                        "geometry-rules",
                        "labels",
                        "lifecycle",
                        "mermaid-everyday",
                        "reference",
                        "repair",
                        "sequence",
                        "workflow"
                      ]
                    },
                    "scenario": {
                      "type": "string",
                      "description": "What the diagram is for, e.g. \"order lifecycle from cart to refund\"."
                    }
                  },
                  "additionalProperties": false
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
        description: "Ask the person watching a structured question. Use it when a choice is theirs to make — an approach, a trade-off, a name — rather than guessing and writing something they did not ask for. Prefer register_question: it returns instantly and cannot time out, and the dialog is shown the moment your turn ends. Use ask_user_question only when you cannot stop — when the answer is needed in the middle of a sequence you are holding open — because that call blocks until they answer.",
        tools: &[
            ToolSpec {
                name: "register_question",
                description: "Register one to four multiple-choice questions to put to the user, and return immediately. HARD REQUIREMENT: after this call you MUST end your turn at once. Do not call another tool, do not read another file, do not keep working. The dialog is only shown to the user when your turn ends, and their answer arrives as your next turn — so anything you do after registering delays the question and is thrown-away work. Register last, say in one short message what you are waiting on, and stop. Each question shows two to four options you wrote; 'Other' is always offered beside them and is never one of yours, so do not write it. The registration is for this turn only: it is raised when the turn ends, and dropped if the turn fails. You are given the id it was filed under; you do not need to remember it.",
                schema: ASK_QUESTIONS_SCHEMA,
            },
            ToolSpec {
                name: "ask_user_question",
                description: "Put one to four multiple-choice questions to the user and wait for their reply. This call blocks for as long as the user takes, so use it only when you cannot stop and come back — otherwise register_question, which cannot time out. Each question shows two to four options you wrote; 'Other' is always offered beside them and is never one of yours, so do not write it. The answer names the options the user picked by their labels, plus anything they typed. If the user would rather talk it through, the result says so and they will say the rest in the chat — carry on from the conversation, do not ask again.",
                schema: ASK_QUESTIONS_SCHEMA,
            },
        ],
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
            ATTACH_DOCUMENT,
            DETACH_DOCUMENT,
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
                        "kind": {"type": "string", "description": "A kind from list_agent_kinds, by name. Omit for the mission's default kind. Use 'custom' only with a definition."},
                        "definition": {"type": "string", "description": "A saved definition, for kind 'custom' only. Never a harness, an account or anything secret."},
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
    ServerSpec {
        name: UBIQ_SQL_READ,
        title: "SQL (read)",
        description: "Read the project's databases: the connections the user let agents use, queried read-only. Look before you write SQL (list_objects, describe_table), ask for few rows and short fields, and fetch the one cell you need whole with get_blob. A named editor lets you and the user work on a query together.",
        tools: &[
            SQL_LIST_CONNECTIONS,
            SQL_LIST_OBJECTS,
            SQL_DESCRIBE_TABLE,
            SQL_EXPORT_DBML,
            SQL_QUERY,
            SQL_GET_BLOB,
            SQL_OPEN_EDITOR,
            SQL_READ_EDITOR,
            SQL_EDIT_EDITOR,
            SQL_RUN_EDITOR,
        ],
    },
    ServerSpec {
        name: UBIQ_SQL_WRITE,
        title: "SQL (read and write)",
        description: "Read and change the project's databases, on the connections the user marked read-write. Everything the read server does, plus execute. Look before you write (list_objects, describe_table), keep statements narrow, and use a named editor when the user should see a change before it runs.",
        tools: &[
            SQL_LIST_CONNECTIONS,
            SQL_LIST_OBJECTS,
            SQL_DESCRIBE_TABLE,
            SQL_EXPORT_DBML,
            SQL_QUERY,
            SQL_EXECUTE,
            SQL_GET_BLOB,
            SQL_OPEN_EDITOR,
            SQL_READ_EDITOR,
            SQL_EDIT_EDITOR,
            SQL_RUN_EDITOR,
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
    fn only_the_write_sql_server_has_execute_and_both_require_their_sizes() {
        let tools =
            |name: &str| -> Vec<&'static ToolSpec> { server(name).unwrap().tools.iter().collect() };
        assert!(!tools(UBIQ_SQL_READ).iter().any(|t| t.name == "execute"));
        assert!(tools(UBIQ_SQL_WRITE).iter().any(|t| t.name == "execute"));
        for name in [UBIQ_SQL_READ, UBIQ_SQL_WRITE] {
            for tool in tools(name) {
                if matches!(tool.name, "query" | "execute" | "run_editor") {
                    let schema: Value = serde_json::from_str(tool.schema).unwrap();
                    let required = schema["required"].as_array().unwrap();
                    for field in ["max_rows", "max_field_size"] {
                        assert!(required.iter().any(|r| r == field), "{name}/{}", tool.name);
                    }
                }
            }
        }
    }

    #[test]
    fn the_sql_servers_are_in_no_default_set() {
        for set in [COORDINATOR_MCPS, WORKER_MCPS] {
            assert!(!set.contains(&UBIQ_SQL_READ) && !set.contains(&UBIQ_SQL_WRITE));
        }
    }

    #[test]
    fn the_archify_server_agrees_with_its_engine_and_is_in_no_default_set() {
        for set in [COORDINATOR_MCPS, WORKER_MCPS] {
            assert!(!set.contains(&UBIQ_ARCHIFY));
        }
        let spec = server(UBIQ_ARCHIFY).unwrap();
        let schema = |tool: &str, field: &str| -> Vec<String> {
            let tool = spec.tools.iter().find(|t| t.name == tool).unwrap();
            let schema: Value = serde_json::from_str(tool.schema).unwrap();
            schema["properties"][field]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect()
        };
        let types: Vec<String> = ubiq_archify::schema::DOC_TYPES.iter().map(|s| s.to_string()).collect();
        assert_eq!(schema("archify_new", "type"), types);
        assert_eq!(schema("archify_validate", "type"), types);
        let topics: Vec<String> = ubiq_archify::guide::topic_names().iter().map(|s| s.to_string()).collect();
        assert_eq!(schema("archify_guide", "topic"), topics);
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
