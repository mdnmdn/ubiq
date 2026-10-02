//! The service, driven through its messages against a temporary SQLite file: no network, no
//! keychain, and the answers read off a real bus client.

use std::time::{Duration, Instant};

use tempfile::TempDir;
use ubiq_db::conn::DbKind;
use ubiq_db::value::Value;
use ubiq_proto::bus::{Client, HostEnd, Hub, To, hub};
use ubiq_proto::db::{
    ColumnMeta, DbAgentAccess, DbListing, DbNode, DbOutcome, DbRun, DbRunOptions, ObjectKind,
    PasswordState, TableRef,
};
use ubiq_proto::ids::DbQueryId;
use ubiq_proto::messages::Secret;

use super::secrets::MemoryKeys;
use super::*;

const WAIT: Duration = Duration::from_secs(30);

struct Rig {
    db: Db,
    keys: Arc<MemoryKeys>,
    client: Client,
    host: HostEnd,
    _hub: Hub,
    dir: TempDir,
    project: ProjectId,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let keys = Arc::new(MemoryKeys::new());
        let (hub, host) = hub();
        let client = hub.connect();
        Self {
            db: Db::with_keys(dir.path().join("config"), keys.clone()),
            keys,
            client,
            host,
            _hub: hub,
            dir,
            project: ProjectId::generate(),
        }
    }

    fn project_path(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    fn ctx(&self) -> Ctx {
        Ctx {
            client: self.client.id(),
            asker: self.host.mailbox(To::Client(self.client.id())),
            everyone: self.host.mailbox(To::Everyone),
            project_path: self.project_path(),
        }
    }

    fn send(&self, message: Message) {
        self.db.handle(self.ctx(), message);
    }

    /// The first message the host sends that `pick` wants; the rest are skipped.
    fn next<T>(&self, mut pick: impl FnMut(Message) -> Option<T>) -> T {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let message = self
                .client
                .from_host()
                .recv_timeout(left)
                .expect("the host answered in time");
            if let Some(found) = pick(message) {
                return found;
            }
        }
    }

    fn sqlite_config(&self, name: &str, read_only: bool) -> ConnectionConfig {
        let path = self.dir.path().join(format!("{name}.sqlite"));
        if !path.exists() {
            driver::create_sqlite_database(&path).unwrap();
        }
        let mut config = ConnectionConfig::new(DbKind::Sqlite);
        config.name = name.into();
        config.path = Some(path);
        config.read_only = read_only;
        config
    }

    fn save(
        &self,
        id: Option<DbConnId>,
        config: ConnectionConfig,
        password: SecretEdit,
    ) -> Vec<DbConnection> {
        self.save_agent(id, config, password, None)
    }

    fn save_agent(
        &self,
        id: Option<DbConnId>,
        config: ConnectionConfig,
        password: SecretEdit,
        agent: Option<DbAgentSettings>,
    ) -> Vec<DbConnection> {
        self.send(Message::SaveDbConnection {
            project_id: self.project,
            id,
            config: Box::new(config),
            password,
            remember: true,
            agent,
        });
        self.listed()
    }

    fn agents(&self) -> AgentDb {
        self.db.agent_handle(self.host.mailbox(To::Everyone))
    }

    /// [`Self::people`], opened to agents at `access`.
    fn people_for_agents(&self, access: DbAgentAccess) -> DbConnId {
        let conn = self.people();
        self.set_access(conn, access);
        conn
    }

    fn set_access(&self, conn: DbConnId, access: DbAgentAccess) {
        self.save_agent(
            Some(conn),
            self.sqlite_config("people", false),
            SecretEdit::Keep,
            Some(agent(access, false, "the people")),
        );
    }

    fn agent_run(
        &self,
        conn: DbConnId,
        statements: &[&str],
        read_only: bool,
        rows: usize,
    ) -> Vec<StmtResult> {
        self.agents().run(
            "agent-a",
            self.project,
            &self.project_path(),
            AgentRun {
                conn,
                database: None,
                statements: statements.iter().map(|s| s.to_string()).collect(),
                read_only,
                timeout: Duration::from_secs(20),
                row_limit: rows,
                panel: None,
            },
        )
    }

    fn listed(&self) -> Vec<DbConnection> {
        self.next(|m| match m {
            Message::DbConnectionsListed { connections, .. } => Some(connections),
            _ => None,
        })
    }

    /// A saved read-write SQLite connection, with `person (id integer primary key, name text)`
    /// and two rows in it.
    fn people(&self) -> DbConnId {
        let saved = self.save(None, self.sqlite_config("people", false), SecretEdit::Keep);
        let conn = saved[0].id;
        let session = DbSessionId::generate();
        let results = self.run(
            conn,
            session,
            &[
                "CREATE TABLE person (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
                "INSERT INTO person (name) VALUES ('ada'), ('grace')",
            ],
            DbRunOptions::default(),
        );
        assert!(results.iter().all(Result::is_ok), "{results:?}");
        conn
    }

    fn run(
        &self,
        conn: DbConnId,
        session: DbSessionId,
        statements: &[&str],
        opts: DbRunOptions,
    ) -> Vec<Result<DbOutcome, DbFailure>> {
        let query = DbQueryId::generate();
        self.send(Message::DbQuery {
            project_id: self.project,
            conn,
            session,
            query,
            database: None,
            statements: statements.iter().map(|s| s.to_string()).collect(),
            run: DbRun::Query,
            opts,
        });
        self.collect(query)
    }

    fn collect(&self, query: DbQueryId) -> Vec<Result<DbOutcome, DbFailure>> {
        let mut all = Vec::new();
        loop {
            let (result, last) = self.next(|m| match m {
                Message::DbQueryResult {
                    query: q,
                    result,
                    last,
                    ..
                } if q == query => Some((result.map(|b| *b), last)),
                _ => None,
            });
            all.push(result);
            if last {
                return all;
            }
        }
    }

    fn rows(&self, conn: DbConnId, sql: &str) -> Vec<Vec<Value>> {
        match self
            .run(
                conn,
                DbSessionId::generate(),
                &[sql],
                DbRunOptions::default(),
            )
            .remove(0)
        {
            Ok(DbOutcome::Rows(rows)) => rows.rows,
            other => panic!("expected rows, got {other:?}"),
        }
    }
}

fn person() -> TableRef {
    TableRef::new(Some("main"), None, "person")
}

fn agent(access: DbAgentAccess, default: bool, description: &str) -> DbAgentSettings {
    DbAgentSettings {
        access,
        default,
        description: description.into(),
    }
}

#[test]
fn connections_round_trip_without_the_password_in_the_file_or_the_listing() {
    let rig = Rig::new();
    let saved = rig.save(
        None,
        rig.sqlite_config("local", false),
        SecretEdit::Set(Secret::new("hunter2")),
    );
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].config.name, "local");
    assert_eq!(saved[0].config.password, None);
    assert_eq!(saved[0].password, PasswordState::Saved);
    assert_eq!(
        rig.db.inner.secrets.keystore(),
        ubiq_proto::db::DbKeystore::Ready
    );

    let file = rig.db.inner.dirs.data(rig.project).db_connections();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains("local") && !text.contains("hunter2"),
        "{text}"
    );

    // A second service over the same root and the same keychain reads the same list.
    let again = Db::with_keys(rig.dir.path().join("config"), rig.keys.clone());
    again.handle(
        rig.ctx(),
        Message::DbConnections {
            project_id: rig.project,
        },
    );
    let listed = rig.listed();
    assert_eq!(listed, saved);

    // An edit keeps the id and the password; a delete takes both away.
    let renamed = rig.save(
        Some(saved[0].id),
        rig.sqlite_config("renamed", false),
        SecretEdit::Keep,
    );
    assert_eq!(renamed.len(), 1);
    assert_eq!(renamed[0].id, saved[0].id);
    assert_eq!(renamed[0].config.name, "renamed");
    assert_eq!(renamed[0].password, PasswordState::Saved);
    rig.send(Message::DeleteDbConnection {
        project_id: rig.project,
        id: saved[0].id,
    });
    assert!(rig.listed().is_empty());
    assert_eq!(
        rig.db.inner.secrets.state(rig.project, saved[0].id),
        PasswordState::None
    );
}

#[test]
fn the_tree_lists_databases_objects_and_columns() {
    let rig = Rig::new();
    let conn = rig.people();
    let tree = |node: DbNode| {
        rig.send(Message::DbTree {
            project_id: rig.project,
            conn,
            node,
        });
        rig.next(|m| match m {
            Message::DbTreeListing { result, .. } => Some(result),
            _ => None,
        })
        .expect("a listing")
    };
    let DbListing::Names(databases) = tree(DbNode::Databases) else {
        panic!("names")
    };
    assert!(databases.contains(&"main".to_string()));
    let DbListing::Objects(objects) = tree(DbNode::Objects {
        database: "main".into(),
        schema: None,
    }) else {
        panic!("objects")
    };
    assert!(
        objects
            .iter()
            .any(|o| o.name == "person" && o.kind == ObjectKind::Table)
    );
    let DbListing::Columns(columns) = tree(DbNode::Columns { table: person() }) else {
        panic!("columns")
    };
    assert_eq!(
        columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["id", "name"]
    );
    assert!(columns[0].is_pk);
}

#[test]
fn a_table_page_carries_rows_columns_and_the_exact_count() {
    let rig = Rig::new();
    let conn = rig.people();
    let page = |filter: &str, count: bool| {
        let query = DbQueryId::generate();
        rig.send(Message::DbTablePage {
            project_id: rig.project,
            conn,
            session: DbSessionId::generate(),
            query,
            table: Box::new(person()),
            filter: filter.into(),
            order_by: "id desc".into(),
            limit: 1,
            offset: 0,
            count,
            read_only: true,
        });
        rig.next(|m| match m {
            Message::DbTablePageResult {
                query: q, result, ..
            } if q == query => Some(result),
            _ => None,
        })
    };
    let first = page("", true).expect("a page");
    assert_eq!(
        first.rows.rows,
        vec![vec![Value::Int(2), Value::Text("grace".into())]]
    );
    assert_eq!(first.exact_count, Some(2));
    assert_eq!(first.columns.len(), 2);
    assert!(first.columns[0].is_pk);

    // A fragment that is not one read statement is refused before it is sent anywhere.
    let refused = page("1=1; DELETE FROM person", false).unwrap_err();
    assert_eq!(refused.kind, DbFailureKind::Query);
    assert_eq!(
        rig.rows(conn, "SELECT count(*) FROM person"),
        vec![vec![Value::Int(2)]]
    );
}

#[test]
fn a_query_returns_rows_and_counts_a_write() {
    let rig = Rig::new();
    let conn = rig.people();
    let session = DbSessionId::generate();
    let results = rig.run(
        conn,
        session,
        &[
            "UPDATE person SET name = 'ADA' WHERE id = 1",
            "SELECT name FROM person ORDER BY id",
            "SELECT nope FROM person",
            "SELECT 'never runs'",
        ],
        DbRunOptions::default(),
    );
    // The failure ends the run: three replies, not four.
    assert_eq!(results.len(), 3);
    assert!(matches!(results[0], Ok(DbOutcome::Affected(1))));
    let Ok(DbOutcome::Rows(rows)) = &results[1] else {
        panic!("rows")
    };
    assert_eq!(rows.rows[0], vec![Value::Text("ADA".into())]);
    assert_eq!(results[2].as_ref().unwrap_err().kind, DbFailureKind::Query);
}

#[test]
fn a_read_only_run_is_refused_by_the_parser_before_the_server_sees_it() {
    let rig = Rig::new();
    let conn = rig.people();
    let opts = DbRunOptions {
        read_only: true,
        ..DbRunOptions::default()
    };
    let results = rig.run(conn, DbSessionId::generate(), &["DELETE FROM person"], opts);
    let refused = results[0].as_ref().unwrap_err();
    assert_eq!(refused.kind, DbFailureKind::ReadOnly);
    assert!(refused.message.contains("read-only"), "{}", refused.message);
    assert_eq!(
        rig.rows(conn, "SELECT count(*) FROM person"),
        vec![vec![Value::Int(2)]]
    );

    // A read-only *connection* makes every run read-only, whatever the message says.
    let ro = rig.save(None, rig.sqlite_config("people", true), SecretEdit::Keep);
    let ro = ro.iter().find(|c| c.config.read_only).unwrap().id;
    let results = rig.run(
        ro,
        DbSessionId::generate(),
        &["INSERT INTO person (name) VALUES ('x')"],
        DbRunOptions::default(),
    );
    assert_eq!(
        results[0].as_ref().unwrap_err().kind,
        DbFailureKind::ReadOnly
    );
    // And the edits path says no before rendering anything.
    let columns = ColumnMeta::new(DbKind::Sqlite, "name", "TEXT");
    let session = DbSessionId::generate();
    let query = DbQueryId::generate();
    rig.send(Message::DbApplyEdits {
        project_id: rig.project,
        conn: ro,
        session,
        query,
        table: Box::new(person()),
        edits: vec![ubiq_proto::db::RowEdit::Insert {
            values: vec![(columns, Value::Text("x".into()))],
        }],
    });
    let applied = rig.next(|m| match m {
        Message::DbEditsApplied { result, .. } => Some(result),
        _ => None,
    });
    assert_eq!(applied.unwrap_err().failure.kind, DbFailureKind::ReadOnly);
}

#[test]
fn a_batch_of_edits_is_one_transaction_and_a_failure_rolls_all_of_it_back() {
    let rig = Rig::new();
    let conn = rig.people();
    rig.send(Message::DbTree {
        project_id: rig.project,
        conn,
        node: DbNode::Columns { table: person() },
    });
    let DbListing::Columns(columns) = rig
        .next(|m| match m {
            Message::DbTreeListing { result, .. } => Some(result),
            _ => None,
        })
        .unwrap()
    else {
        panic!("columns")
    };
    let insert = |id: i64, name: &str| ubiq_proto::db::RowEdit::Insert {
        values: vec![
            (columns[0].clone(), Value::Int(id)),
            (columns[1].clone(), Value::Text(name.into())),
        ],
    };
    let apply = |edits: Vec<ubiq_proto::db::RowEdit>| {
        rig.send(Message::DbApplyEdits {
            project_id: rig.project,
            conn,
            session: DbSessionId::generate(),
            query: DbQueryId::generate(),
            table: Box::new(person()),
            edits,
        });
        rig.next(|m| match m {
            Message::DbEditsApplied { result, .. } => Some(result),
            _ => None,
        })
    };

    // The second insert repeats the key: the first one goes with it.
    let failed = apply(vec![insert(10, "linus"), insert(10, "again")]).unwrap_err();
    assert_eq!(failed.index, 1);
    assert!(failed.statement.contains("again"), "{}", failed.statement);
    assert_eq!(failed.failure.kind, DbFailureKind::Query);
    assert_eq!(
        rig.rows(conn, "SELECT count(*) FROM person"),
        vec![vec![Value::Int(2)]]
    );

    assert_eq!(apply(vec![insert(10, "linus"), insert(11, "ken")]), Ok(2));
    assert_eq!(
        rig.rows(conn, "SELECT count(*) FROM person"),
        vec![vec![Value::Int(4)]]
    );
}

#[test]
fn a_long_query_is_stopped_by_its_cancel() {
    let rig = Rig::new();
    let conn = rig.people();
    let session = DbSessionId::generate();
    let query = DbQueryId::generate();
    rig.send(Message::DbQuery {
        project_id: rig.project,
        conn,
        session,
        query,
        database: None,
        // Never ends on its own.
        statements: vec![
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c"
                .into(),
        ],
        run: DbRun::Query,
        // A backstop, so a cancel that did nothing fails the test as a timeout, not a hang.
        opts: DbRunOptions {
            timeout_ms: Some(20_000),
            ..DbRunOptions::default()
        },
    });
    std::thread::sleep(Duration::from_millis(400));
    let stopped_at = Instant::now();
    rig.send(Message::DbCancel {
        project_id: rig.project,
        session,
        query,
    });
    let results = rig.collect(query);
    assert_eq!(
        results[0].as_ref().unwrap_err().kind,
        DbFailureKind::Cancelled
    );
    assert!(stopped_at.elapsed() < Duration::from_secs(10));

    // The session survives it.
    let again = rig.run(conn, session, &["SELECT 1"], DbRunOptions::default());
    assert!(again[0].is_ok());
    assert_eq!(rig.db.inner.runs.in_flight(), 0);
}

#[test]
fn a_password_that_cannot_be_opened_is_a_prompt_and_the_answer_connects() {
    let rig = Rig::new();
    let conn = rig.save(
        None,
        rig.sqlite_config("local", false),
        SecretEdit::Set(Secret::new("hunter2")),
    )[0]
    .id;
    rig.keys.lose();
    rig.send(Message::DbTree {
        project_id: rig.project,
        conn,
        node: DbNode::Databases,
    });
    let result = rig.next(|m| match m {
        Message::DbTreeListing { result, .. } => Some(result),
        _ => None,
    });
    assert_eq!(result.unwrap_err().kind, DbFailureKind::NeedsPassword);

    rig.send(Message::DbPassword {
        project_id: rig.project,
        conn,
        password: Secret::new("hunter3"),
        remember: false,
    });
    let state = rig.next(|m| match m {
        Message::DbConnectionState {
            state: state @ (DbConnState::Connected { .. } | DbConnState::Failed(_)),
            ..
        } => Some(state),
        _ => None,
    });
    assert!(matches!(state, DbConnState::Connected { .. }), "{state:?}");
    assert_eq!(
        rig.db.inner.secrets.state(rig.project, conn),
        PasswordState::Session
    );
}

#[test]
fn a_test_reports_the_server_and_a_new_file_is_created_once() {
    let rig = Rig::new();
    let probe = DbProbeId::generate();
    rig.send(Message::TestDbConnection {
        project_id: rig.project,
        probe,
        id: None,
        config: Box::new(rig.sqlite_config("probe", false)),
        password: SecretEdit::Keep,
    });
    let tested = rig.next(|m| match m {
        Message::DbTested {
            probe: p, result, ..
        } if p == probe => Some(result),
        _ => None,
    });
    assert!(!tested.expect("connects").is_empty());

    let path = rig
        .dir
        .path()
        .join("fresh.sqlite")
        .to_string_lossy()
        .into_owned();
    for expect_created in [true, false] {
        rig.send(Message::CreateDbFile {
            project_id: rig.project,
            path: path.clone(),
        });
        let created = rig.next(|m| match m {
            Message::DbFileCreated { .. } => Some(true),
            Message::DbFileError { .. } => Some(false),
            _ => None,
        });
        assert_eq!(created, expect_created);
    }
}

#[test]
fn closing_a_window_closes_its_sessions() {
    let rig = Rig::new();
    rig.people();
    assert!(rig.db.session_count() >= 1);
    rig.db.client_gone(rig.client.id());
    assert_eq!(rig.db.session_count(), 0);
}

// ---- agents -------------------------------------------------------------------------------------

#[test]
fn agent_settings_keep_their_invariants_on_save() {
    let rig = Rig::new();
    let access = |listed: &[DbConnection], id: DbConnId| {
        let c = listed.iter().find(|c| c.id == id).unwrap();
        (c.agent.access, c.agent.default)
    };
    let a = rig.save_agent(
        None,
        rig.sqlite_config("a", false),
        SecretEdit::Keep,
        Some(agent(DbAgentAccess::Rw, true, "first")),
    )[0]
    .id;

    // Made read-only: `rw` becomes `ro`, and `None` keeps the rest of what is saved.
    let listed = rig.save(Some(a), rig.sqlite_config("a", true), SecretEdit::Keep);
    assert_eq!(access(&listed, a), (DbAgentAccess::Ro, true));
    assert_eq!(listed[0].agent.description, "first");

    // A second default takes it from the first.
    let listed = rig.save_agent(
        None,
        rig.sqlite_config("b", false),
        SecretEdit::Keep,
        Some(agent(DbAgentAccess::Rw, true, "")),
    );
    let b = listed.iter().find(|c| c.config.name == "b").unwrap().id;
    assert_eq!(access(&listed, a), (DbAgentAccess::Ro, false));
    assert_eq!(access(&listed, b), (DbAgentAccess::Rw, true));

    // A default agents cannot see is no default, and takes it from nobody.
    let listed = rig.save_agent(
        None,
        rig.sqlite_config("c", false),
        SecretEdit::Keep,
        Some(agent(DbAgentAccess::None, true, "")),
    );
    let c = listed.iter().find(|c| c.config.name == "c").unwrap().id;
    assert_eq!(access(&listed, c), (DbAgentAccess::None, false));
    assert_eq!(access(&listed, b), (DbAgentAccess::Rw, true));

    // Only the two open to agents are listed to them.
    let seen: Vec<_> = rig
        .agents()
        .connections(rig.project, &rig.project_path())
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(seen, [a, b]);
}

#[test]
fn the_access_overlay_is_local_and_the_description_shared() {
    let rig = Rig::new();
    let id = rig.save_agent(
        None,
        rig.sqlite_config("a", false),
        SecretEdit::Keep,
        Some(agent(DbAgentAccess::Ro, true, "orders, read them")),
    )[0]
    .id;
    let data = rig.db.inner.dirs.data(rig.project);
    let shared = std::fs::read_to_string(data.db_connections()).unwrap();
    assert!(
        shared.contains("orders, read them") && !shared.contains("access"),
        "{shared}"
    );
    let local = std::fs::read_to_string(rig.db.inner.files(rig.project).agents).unwrap();
    assert!(
        rig.db.inner.files(rig.project).agents.ends_with(format!(
            "projects/{}/local/{}",
            rig.project,
            store::AGENTS_FILE
        ))
    );
    assert!(
        local.contains(&id.to_string()) && local.contains("access = \"ro\""),
        "{local}"
    );

    // Another service over the same root reads both halves back.
    let again = Db::with_keys(rig.dir.path().join("config"), rig.keys.clone());
    again.handle(
        rig.ctx(),
        Message::DbConnections {
            project_id: rig.project,
        },
    );
    assert_eq!(
        rig.listed()[0].agent,
        agent(DbAgentAccess::Ro, true, "orders, read them")
    );
}

#[test]
fn a_read_only_agent_run_refuses_an_update() {
    let rig = Rig::new();
    let conn = rig.people_for_agents(DbAgentAccess::Rw);
    let refused = rig.agent_run(conn, &["UPDATE person SET name = 'x'"], true, 10);
    assert_eq!(refused.len(), 1);
    assert_eq!(
        refused[0].outcome.as_ref().unwrap_err().kind,
        DbFailureKind::ReadOnly
    );
    assert_eq!(refused[0].kind, "UPDATE");

    // At `ro`, even a run that asks to write is refused before anything is queued.
    rig.set_access(conn, DbAgentAccess::Ro);
    let refused = rig.agent_run(conn, &["UPDATE person SET name = 'x'"], false, 10);
    assert_eq!(
        refused[0].outcome.as_ref().unwrap_err().kind,
        DbFailureKind::ReadOnly
    );
    assert_eq!(
        rig.rows(conn, "SELECT count(*) FROM person WHERE name = 'x'"),
        vec![vec![Value::Int(0)]]
    );

    // At `rw` a write runs and is counted; at `none` the connection is not there at all.
    rig.set_access(conn, DbAgentAccess::Rw);
    let wrote = rig.agent_run(
        conn,
        &["UPDATE person SET name = 'x' WHERE id = 1"],
        false,
        10,
    );
    assert!(
        matches!(wrote[0].outcome, Ok(DbOutcome::Affected(1))),
        "{wrote:?}"
    );
    rig.set_access(conn, DbAgentAccess::None);
    let refused = rig.agent_run(conn, &["SELECT 1"], true, 10);
    assert_eq!(
        refused[0].outcome.as_ref().unwrap_err().kind,
        DbFailureKind::NotFound
    );
}

#[test]
fn an_agent_run_is_capped_at_its_row_limit() {
    let rig = Rig::new();
    let conn = rig.people_for_agents(DbAgentAccess::Ro);
    let results = rig.agent_run(
        conn,
        &[
            "SELECT 1 AS n UNION ALL SELECT 2 UNION ALL SELECT 3",
            "SELECT name FROM person",
        ],
        true,
        2,
    );
    assert_eq!(results.len(), 2);
    let Ok(DbOutcome::Rows(first)) = &results[0].outcome else {
        panic!("rows: {results:?}")
    };
    assert_eq!((first.rows.len(), first.truncated), (2, true));
    let Ok(DbOutcome::Rows(second)) = &results[1].outcome else {
        panic!("rows")
    };
    assert_eq!((second.rows.len(), second.truncated), (2, false));
}

#[test]
fn an_agent_sees_the_tree_of_a_connection_open_to_it() {
    let rig = Rig::new();
    let conn = rig.people_for_agents(DbAgentAccess::Ro);
    let listing = rig
        .agents()
        .tree(
            "agent-a",
            rig.project,
            &rig.project_path(),
            conn,
            DbNode::Columns { table: person() },
            Duration::from_secs(20),
        )
        .expect("columns");
    let DbListing::Columns(columns) = listing else {
        panic!("columns")
    };
    assert_eq!(columns.len(), 2);
}

#[test]
fn editor_modes_detect_a_change_since_the_agents_last_write() {
    let editors = Editors::default();
    let project = ProjectId::generate();
    let conn = DbConnId::generate();
    let (opened, touched) = editors.open("a", "Agent A", project, "q", conn, None, None);
    assert!(opened.created && touched && !opened.changed_since_last_write);
    assert_eq!(opened.editor.rev, 0);

    // The agent writes; nobody else has.
    let (wrote, _) = editors
        .edit(
            "a",
            "Agent A",
            project,
            "q",
            "SELECT 1".into(),
            EditMode::KeepIfUserChanged,
        )
        .unwrap();
    assert!(!wrote.kept && !wrote.changed_since_last_write);
    assert_eq!(
        (wrote.editor.rev, wrote.editor.text.as_str()),
        (1, "SELECT 1")
    );
    assert_eq!(wrote.last_edited_by, Some(EditedBy::Agent("a".into())));

    // The user edits from rev 1: accepted. An edit from rev 1 again is stale.
    let session = wrote.editor.session;
    assert!(matches!(
        editors.user_edit(project, session, "SELECT 2".into(), 1),
        UserEdit::Applied(ref e) if e.rev == 2
    ));
    assert!(matches!(
        editors.user_edit(project, session, "SELECT 3".into(), 1),
        UserEdit::Stale(ref e) if e.text == "SELECT 2"
    ));
    assert_eq!(
        editors.user_edit(project, session, "SELECT 2".into(), 2),
        UserEdit::Unchanged
    );

    // keep_if_user_changed leaves the user's text.
    let (kept, touched) = editors
        .edit(
            "a",
            "Agent A",
            project,
            "q",
            "SELECT 9".into(),
            EditMode::KeepIfUserChanged,
        )
        .unwrap();
    assert!(kept.kept && kept.changed_since_last_write && !touched);
    assert_eq!(kept.editor.text, "SELECT 2");
    assert_eq!(kept.last_edited_by, Some(EditedBy::User));

    // overwrite_return_previous writes and hands back what it replaced.
    let (over, _) = editors
        .edit(
            "a",
            "Agent A",
            project,
            "q",
            "SELECT 9".into(),
            EditMode::OverwriteReturnPrevious,
        )
        .unwrap();
    assert_eq!(over.previous_text.as_deref(), Some("SELECT 2"));
    assert_eq!(
        (over.editor.rev, over.editor.text.as_str()),
        (3, "SELECT 9")
    );

    // ... and with nothing changed since, returns nothing; force says whether others had.
    let (again, _) = editors
        .edit(
            "a",
            "Agent A",
            project,
            "q",
            "SELECT 10".into(),
            EditMode::OverwriteReturnPrevious,
        )
        .unwrap();
    assert_eq!(again.previous_text, None);
    let (forced, _) = editors
        .edit(
            "a",
            "Agent A",
            project,
            "q",
            "SELECT 11".into(),
            EditMode::Force,
        )
        .unwrap();
    assert!(!forced.changed_since_last_write);

    // Another agent that never wrote sees the editor changed; its own namespace is separate.
    let (other, _) = editors.open("b", "Agent B", project, "q", conn, None, None);
    assert!(other.created && other.editor.session != session);
    assert!(
        editors
            .read("a", project, "q")
            .is_some_and(|r| !r.changed_since_last_write)
    );
    assert!(
        editors
            .edit("c", "C", project, "q", String::new(), EditMode::Force)
            .is_none()
    );
    assert_eq!(editors.list(project).len(), 2);
}

#[test]
fn an_agents_editor_run_is_shown_to_everyone_and_any_window_may_stop_it() {
    let rig = Rig::new();
    let conn = rig.people_for_agents(DbAgentAccess::Ro);
    let agents = rig.agents();
    let opened = agents.open_editor(
        "agent-a",
        "Agent A",
        rig.project,
        "people",
        conn,
        None,
        Some("SELECT name FROM person ORDER BY id; SELECT 2".into()),
    );
    let announced = rig.next(|m| match m {
        Message::DbEditorChanged { editor, reveal, .. } => Some((editor, reveal)),
        _ => None,
    });
    assert_eq!(announced.0.text, opened.editor.text);
    assert!(announced.1, "a new editor's tab is revealed");

    // The window lists it.
    rig.send(Message::DbEditors {
        project_id: rig.project,
    });
    let listed = rig.next(|m| match m {
        Message::DbEditorsListed { editors, .. } => Some(editors),
        _ => None,
    });
    assert_eq!(listed.len(), 1);

    let results = agents
        .run_editor(
            "agent-a",
            rig.project,
            &rig.project_path(),
            "people",
            true,
            Duration::from_secs(20),
            10,
        )
        .expect("ran");
    assert_eq!(results.len(), 2);
    let session = opened.editor.session;
    let query = rig.next(|m| match m {
        Message::DbAgentRun {
            session: s,
            query,
            statements,
            ..
        } if s == session => {
            assert_eq!(statements.len(), 2);
            Some(query)
        }
        _ => None,
    });
    let shown = rig.collect(query);
    assert_eq!(shown.len(), 2);
    assert!(shown.iter().all(Result::is_ok));

    // A long agent run in the editor, stopped by a window that did not start it.
    let long = {
        let agents = agents.clone();
        let project = rig.project;
        let path = rig.project_path();
        std::thread::spawn(move || {
            agents.run(
                "agent-a",
                project,
                &path,
                AgentRun {
                    conn,
                    database: None,
                    statements: vec![
                        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) \
                         SELECT count(*) FROM c"
                            .into(),
                    ],
                    read_only: true,
                    timeout: Duration::from_secs(20),
                    row_limit: 1,
                    panel: Some(session),
                },
            )
        })
    };
    let query = rig.next(|m| match m {
        Message::DbAgentRun { query, .. } => Some(query),
        _ => None,
    });
    std::thread::sleep(Duration::from_millis(400));
    let stopped_at = Instant::now();
    rig.send(Message::DbCancel {
        project_id: rig.project,
        session,
        query,
    });
    let results = long.join().unwrap();
    assert_eq!(
        results[0].outcome.as_ref().unwrap_err().kind,
        DbFailureKind::Cancelled
    );
    assert!(stopped_at.elapsed() < Duration::from_secs(10));
    assert_eq!(rig.db.inner.runs.in_flight(), 0);
}
