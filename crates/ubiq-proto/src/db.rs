//! A project's databases: the records the database family carries, and the model they are made of.
//!
//! The model — a connection's configuration, a typed cell, a result set, a table reference, a row
//! edit, a plan — is defined once, in `ubiq-db`, and re-exported here; defining it a second time
//! for the wire is the drift this avoids. What is below is the contract's own: how a saved
//! connection is told to the interface, how a password is edited without ever being read back, which
//! node of the structure tree is asked for, how one statement ran, and why one did not.
//!
//! **Nothing here holds a decrypted password towards the interface.** A [`DbConnection`] carries a
//! [`PasswordState`] and its `config.password` is always `None`; plaintext crosses the bus only
//! towards the host, in a [`crate::messages::Secret`].

use serde::{Deserialize, Serialize};

use crate::ids::{DbConnId, DbSessionId};
use crate::messages::Secret;

pub use ubiq_db::conn::{ConnectionConfig, DbKind, SslMode};
pub use ubiq_db::dbml::StructureScope;
pub use ubiq_db::edit::RowEdit;
pub use ubiq_db::value::{DataType, Value};
pub use ubiq_db::{
    ColumnMeta, DbObject, ExecOutcome, ObjectKind, Plan, PlanFormat, PlanNode, ResultSet, TableRef,
};

/// Whether the host holds a password for a connection, and where.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PasswordState {
    /// None filed and none needed — a file database, integrated auth, or a server that asks for no
    /// password.
    #[default]
    None,
    /// Filed, sealed under this install's key.
    Saved,
    /// Was filed, and cannot be opened: the key is gone or the ciphertext does not verify. The next
    /// use answers [`DbConnState::NeedsPassword`] — a prompt, never an error.
    Missing,
    /// Held in the host's memory for this run only: the keychain was unusable, or the user chose
    /// not to remember it.
    Session,
}

/// One saved connection as the interface is told about it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DbConnection {
    pub id: DbConnId,
    /// `config.password` is always `None` here, towards the interface.
    pub config: ConnectionConfig,
    pub password: PasswordState,
    /// What an agent may do with this connection. Absent in an older record: no access.
    #[serde(default)]
    pub agent: DbAgentSettings,
}

/// What an agent may do with a connection, through the SQL MCP servers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbAgentAccess {
    /// Invisible to agents.
    #[default]
    None,
    /// Read-only queries.
    Ro,
    /// Read and write.
    Rw,
}

/// A connection's agent-facing settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbAgentSettings {
    #[serde(default)]
    pub access: DbAgentAccess,
    /// The connection an agent gets when it names none. At most one per project, and never at
    /// [`DbAgentAccess::None`].
    #[serde(default)]
    pub default: bool,
    /// What the connection is for, as an agent reads it.
    #[serde(default)]
    pub description: String,
}

/// A named query editor shared between the user and an agent. Lives in the host's memory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbEditor {
    pub name: String,
    /// The session its statements run in.
    pub session: DbSessionId,
    pub conn: DbConnId,
    pub database: Option<String>,
    pub text: String,
    /// Bumped on every change to `text`.
    pub rev: u64,
    /// The agent that controls it, as the host keys agents; `None` for a user's own editor.
    pub agent_key: Option<String>,
    /// That agent's title, for the mark on the tab.
    pub agent_title: Option<String>,
}

/// What a form does to a connection's password. The password field is write-only: it sends
/// `Keep` unless the user typed (`Set`) or pressed *Forget password* (`Clear`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecretEdit {
    /// Leave the saved one as it is. In a Test, use the saved one host-side.
    Keep,
    /// Replace it. Material, so a [`Secret`]: never printed.
    Set(Secret),
    /// Forget it.
    Clear,
}

/// Whether this install can seal a password at all.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DbKeystore {
    Ready,
    /// The OS keychain is unusable; nothing is written to disk and a typed password lives in the
    /// host's memory for the run. The sentence says why.
    Unavailable(String),
}

/// A node of the structure tree the interface asks the children of.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DbNode {
    /// The top level of the connection.
    Databases,
    Schemas {
        database: String,
    },
    /// Tables, views and the rest of one database; `schema` is `None` exactly when the engine has
    /// no schema level.
    Objects {
        database: String,
        schema: Option<String>,
    },
    Columns {
        table: TableRef,
    },
}

/// The children of a [`DbNode`], in the shape that node has.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DbListing {
    /// Answer to `Databases` and `Schemas`.
    Names(Vec<String>),
    /// Answer to `Objects`.
    Objects(Vec<DbObject>),
    /// Answer to `Columns`.
    Columns(Vec<ColumnMeta>),
}

/// What a SQL run does with its statements.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DbRun {
    Query,
    Explain { analyze: bool },
}

/// How one run is limited. The host caps `row_limit` and fills the rest in when absent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbRunOptions {
    /// The tab's lock; a read-only connection is read-only whatever this says.
    pub read_only: bool,
    pub timeout_ms: Option<u64>,
    pub row_limit: Option<u32>,
}

/// What one statement produced.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DbOutcome {
    Rows(ResultSet),
    /// Rows affected by a statement that returns none.
    Affected(u64),
    Plan(Plan),
}

/// One page of a table, with the table's own columns (primary key and defaults filled, which a
/// result's columns do not carry).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DbPage {
    pub columns: Vec<ColumnMeta>,
    pub rows: ResultSet,
    /// `count(*)`, only when the page was asked with `count: true` and it finished.
    pub exact_count: Option<u64>,
}

/// Why a database act did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DbFailureKind {
    Config,
    Connect,
    Query,
    ReadOnly,
    Cancelled,
    Timeout,
    /// The connection needs a password the host does not hold. Answered with `DbPassword`.
    NeedsPassword,
    NotFound,
    Disconnected,
    Unsupported,
    /// This host was built without the drivers.
    Unavailable,
}

/// A failure, with the sentence to print.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbFailure {
    pub kind: DbFailureKind,
    pub message: String,
    /// Where in the statement the server pointed, as a character offset, when it did.
    #[serde(default)]
    pub position: Option<u32>,
}

impl DbFailure {
    pub fn new(kind: DbFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            position: None,
        }
    }
}

/// A batch of edits that was rolled back: which statement failed, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbEditFailure {
    /// The failing statement's place in the batch.
    pub index: u32,
    /// The statement as it was sent.
    pub statement: String,
    pub failure: DbFailure,
}

/// Where a connection stands, as the host keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DbConnState {
    Idle,
    Connecting,
    /// `server` is the server's version string.
    Connected { server: String },
    NeedsPassword,
    Failed(DbFailure),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{DbExportId, DbProbeId, DbQueryId, DbSessionId, ProjectId};
    use crate::messages::Message;
    use crate::wire::{decode, encode};

    fn rows() -> ResultSet {
        ResultSet {
            columns: vec![ColumnMeta::new(DbKind::Sqlite, "id", "INTEGER")],
            rows: vec![vec![Value::Int(1)], vec![Value::Null]],
            truncated: true,
        }
    }

    fn failure() -> DbFailure {
        DbFailure {
            kind: DbFailureKind::Query,
            message: "no such table: t".into(),
            position: Some(14),
        }
    }

    /// One of every variant of the family, request and reply, with `Ok` and `Err` where a reply
    /// carries a `Result`.
    fn family() -> Vec<Message> {
        let project_id = ProjectId::generate();
        let conn = DbConnId::generate();
        let session = DbSessionId::generate();
        let query = DbQueryId::generate();
        let table = Box::new(TableRef::new(Some("main"), None, "person"));
        let mut config = ConnectionConfig::new(DbKind::Sqlite);
        config.name = "local".into();
        config.path = Some("data/cache.sqlite".into());
        let editor = DbEditor {
            name: "scratch".into(),
            session,
            conn,
            database: Some("main".into()),
            text: "select 1".into(),
            rev: 4,
            agent_key: Some("agent-1".into()),
            agent_title: Some("Claude".into()),
        };
        vec![
            Message::DbConnections { project_id },
            Message::SaveDbConnection {
                project_id,
                id: None,
                config: Box::new(config.clone()),
                password: SecretEdit::Set(Secret::new("hunter2")),
                remember: true,
                agent: Some(DbAgentSettings {
                    access: DbAgentAccess::Ro,
                    default: true,
                    description: "the cache".into(),
                }),
            },
            Message::DeleteDbConnection {
                project_id,
                id: conn,
            },
            Message::TestDbConnection {
                project_id,
                probe: DbProbeId::generate(),
                id: Some(conn),
                config: Box::new(config.clone()),
                password: SecretEdit::Keep,
            },
            Message::DbPassword {
                project_id,
                conn,
                password: Secret::new("hunter2"),
                remember: false,
            },
            Message::DbTree {
                project_id,
                conn,
                node: DbNode::Objects {
                    database: "main".into(),
                    schema: None,
                },
            },
            Message::DbTablePage {
                project_id,
                conn,
                session,
                query,
                table: table.clone(),
                filter: "id > 1".into(),
                order_by: "id desc".into(),
                limit: 200,
                offset: 400,
                count: true,
                read_only: false,
            },
            Message::DbQuery {
                project_id,
                conn,
                session,
                query,
                database: Some("main".into()),
                statements: vec!["select 1".into(), "select 2".into()],
                run: DbRun::Explain { analyze: true },
                opts: DbRunOptions {
                    read_only: true,
                    timeout_ms: Some(30_000),
                    row_limit: Some(1_000),
                },
            },
            Message::DbApplyEdits {
                project_id,
                conn,
                session,
                query,
                table,
                edits: vec![RowEdit::Insert {
                    values: vec![(
                        ColumnMeta::new(DbKind::Sqlite, "name", "TEXT"),
                        Value::Text("ada".into()),
                    )],
                }],
            },
            Message::DbCancel {
                project_id,
                session,
                query,
            },
            Message::DbCloseSession {
                project_id,
                session,
            },
            Message::DbDisconnect { project_id, conn },
            Message::CreateDbFile {
                project_id,
                path: "/tmp/new.sqlite".into(),
            },
            Message::DbExportDbml {
                project_id,
                conn,
                request: DbExportId::generate(),
                database: Some("app".into()),
                scope: StructureScope {
                    schema: Some("public".into()),
                    tables: vec!["users".into()],
                },
            },
            Message::DbDbmlReady {
                project_id,
                conn,
                request: DbExportId::generate(),
                result: Ok("Table users {\n  id int [pk]\n}\n".into()),
            },
            Message::DbConnectionsListed {
                project_id,
                connections: vec![DbConnection {
                    id: conn,
                    config,
                    password: PasswordState::Saved,
                    agent: DbAgentSettings::default(),
                }],
                keystore: DbKeystore::Unavailable("no secret service".into()),
            },
            Message::DbTested {
                project_id,
                probe: DbProbeId::generate(),
                result: Ok("3.45.0".into()),
            },
            Message::DbTested {
                project_id,
                probe: DbProbeId::generate(),
                result: Err(DbFailure::new(DbFailureKind::Connect, "refused")),
            },
            Message::DbConnectionState {
                project_id,
                conn,
                state: DbConnState::Connected {
                    server: "3.45.0".into(),
                },
            },
            Message::DbConnectionState {
                project_id,
                conn,
                state: DbConnState::Failed(failure()),
            },
            Message::DbTreeListing {
                project_id,
                conn,
                node: DbNode::Columns {
                    table: TableRef::new(None, None, "person"),
                },
                result: Ok(DbListing::Columns(rows().columns)),
            },
            Message::DbTreeListing {
                project_id,
                conn,
                node: DbNode::Databases,
                result: Ok(DbListing::Names(vec!["main".into()])),
            },
            Message::DbTreeListing {
                project_id,
                conn,
                node: DbNode::Schemas {
                    database: "orders".into(),
                },
                result: Err(DbFailure::new(DbFailureKind::NeedsPassword, "password needed")),
            },
            Message::DbTablePageResult {
                project_id,
                session,
                query,
                result: Ok(Box::new(DbPage {
                    columns: rows().columns,
                    rows: rows(),
                    exact_count: Some(12),
                })),
                elapsed_ms: 7,
            },
            Message::DbTablePageResult {
                project_id,
                session,
                query,
                result: Err(failure()),
                elapsed_ms: 7,
            },
            Message::DbQueryResult {
                project_id,
                session,
                query,
                index: 0,
                last: false,
                result: Ok(Box::new(DbOutcome::Rows(rows()))),
                elapsed_ms: 3,
            },
            Message::DbQueryResult {
                project_id,
                session,
                query,
                index: 1,
                last: false,
                result: Ok(Box::new(DbOutcome::Affected(4))),
                elapsed_ms: 3,
            },
            Message::DbQueryResult {
                project_id,
                session,
                query,
                index: 2,
                last: true,
                result: Ok(Box::new(DbOutcome::Plan(Plan {
                    root: PlanNode::new("SCAN person"),
                    raw: "SCAN person".into(),
                    format: PlanFormat::Table,
                }))),
                elapsed_ms: 3,
            },
            Message::DbQueryResult {
                project_id,
                session,
                query,
                index: 3,
                last: true,
                result: Err(DbFailure::new(DbFailureKind::Cancelled, "cancelled")),
                elapsed_ms: 3,
            },
            Message::DbEditsApplied {
                project_id,
                session,
                query,
                result: Ok(2),
            },
            Message::DbEditsApplied {
                project_id,
                session,
                query,
                result: Err(DbEditFailure {
                    index: 1,
                    statement: "UPDATE person SET name = 'x'".into(),
                    failure: failure(),
                }),
            },
            Message::DbFileCreated {
                project_id,
                path: "/tmp/new.sqlite".into(),
            },
            Message::DbFileError {
                project_id,
                path: "/tmp/new.sqlite".into(),
                message: "exists".into(),
            },
            Message::DbEditors { project_id },
            Message::DbEditorEdit {
                project_id,
                session,
                text: "select 1".into(),
                base_rev: 3,
            },
            Message::DbEditorsListed {
                project_id,
                editors: vec![editor.clone()],
            },
            Message::DbEditorChanged {
                project_id,
                editor: Box::new(editor),
                reveal: true,
            },
            Message::DbAgentRun {
                project_id,
                session,
                query,
                statements: vec!["select 1".into()],
            },
        ]
    }

    #[test]
    fn every_database_message_round_trips_over_the_wire() {
        for message in family() {
            let back = decode(&encode(&message).unwrap()).unwrap();
            // `Message` is not `PartialEq`; compare through the JSON the tape already trusts.
            assert_eq!(
                serde_json::to_value(&message).unwrap(),
                serde_json::to_value(&back).unwrap(),
                "{message:?}"
            );
        }
    }

    #[test]
    fn every_database_message_names_its_project() {
        for message in family() {
            assert!(message.project_id().is_some(), "{message:?}");
        }
    }

    #[test]
    fn a_save_carries_its_password_and_prints_none_of_it() {
        let message = family().swap_remove(1);
        assert!(!format!("{message:?}").contains("hunter2"));
        let Message::SaveDbConnection { password, .. } =
            decode(&encode(&message).unwrap()).unwrap()
        else {
            panic!("expected SaveDbConnection");
        };
        assert_eq!(password, SecretEdit::Set(Secret::new("hunter2")));
    }

    #[test]
    fn a_password_edit_never_prints_its_material() {
        let edit = SecretEdit::Set(Secret::new("hunter2"));
        assert!(!format!("{edit:?}").contains("hunter2"));
    }

    #[test]
    fn a_connection_record_without_agent_settings_reads_back_as_no_access() {
        let mut value = serde_json::to_value(DbConnection {
            id: DbConnId::generate(),
            config: ConnectionConfig::new(DbKind::Sqlite),
            password: PasswordState::None,
            agent: DbAgentSettings::default(),
        })
        .unwrap();
        value.as_object_mut().unwrap().remove("agent");
        let back: DbConnection = serde_json::from_value(value).unwrap();
        assert_eq!(back.agent, DbAgentSettings::default());
        assert_eq!(serde_json::to_string(&DbAgentAccess::Rw).unwrap(), r#""rw""#);
    }

    #[test]
    fn a_failure_without_a_position_reads_back_from_a_record_that_omits_it() {
        let back: DbFailure =
            serde_json::from_str(r#"{"kind":"Query","message":"syntax error"}"#).unwrap();
        assert_eq!(back, DbFailure::new(DbFailureKind::Query, "syntax error"));
    }
}
