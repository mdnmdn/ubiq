//! The service, driven through its messages against a temporary SQLite file: no network, no
//! keychain, and the answers read off a real bus client.

use std::time::{Duration, Instant};

use tempfile::TempDir;
use ubiq_db::conn::DbKind;
use ubiq_db::value::Value;
use ubiq_proto::bus::{Client, HostEnd, Hub, To, hub};
use ubiq_proto::db::{
    ColumnMeta, DbListing, DbNode, DbOutcome, DbRun, DbRunOptions, ObjectKind, PasswordState,
    TableRef,
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

    fn save(&self, id: Option<DbConnId>, config: ConnectionConfig, password: SecretEdit) -> Vec<DbConnection> {
        self.send(Message::SaveDbConnection {
            project_id: self.project,
            id,
            config: Box::new(config),
            password,
            remember: true,
        });
        self.listed()
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
            .run(conn, DbSessionId::generate(), &[sql], DbRunOptions::default())
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
    assert_eq!(rig.db.inner.secrets.keystore(), ubiq_proto::db::DbKeystore::Ready);

    let file = rig.db.inner.dirs.data(rig.project).db_connections();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("local") && !text.contains("hunter2"), "{text}");

    // A second service over the same root and the same keychain reads the same list.
    let again = Db::with_keys(rig.dir.path().join("config"), rig.keys.clone());
    again.handle(rig.ctx(), Message::DbConnections { project_id: rig.project });
    let listed = rig.listed();
    assert_eq!(listed, saved);

    // An edit keeps the id and the password; a delete takes both away.
    let renamed = rig.save(Some(saved[0].id), rig.sqlite_config("renamed", false), SecretEdit::Keep);
    assert_eq!(renamed.len(), 1);
    assert_eq!(renamed[0].id, saved[0].id);
    assert_eq!(renamed[0].config.name, "renamed");
    assert_eq!(renamed[0].password, PasswordState::Saved);
    rig.send(Message::DeleteDbConnection {
        project_id: rig.project,
        id: saved[0].id,
    });
    assert!(rig.listed().is_empty());
    assert_eq!(rig.db.inner.secrets.state(rig.project, saved[0].id), PasswordState::None);
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
    assert!(objects
        .iter()
        .any(|o| o.name == "person" && o.kind == ObjectKind::Table));
    let DbListing::Columns(columns) = tree(DbNode::Columns { table: person() }) else {
        panic!("columns")
    };
    assert_eq!(columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "name"]);
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
            Message::DbTablePageResult { query: q, result, .. } if q == query => Some(result),
            _ => None,
        })
    };
    let first = page("", true).expect("a page");
    assert_eq!(first.rows.rows, vec![vec![Value::Int(2), Value::Text("grace".into())]]);
    assert_eq!(first.exact_count, Some(2));
    assert_eq!(first.columns.len(), 2);
    assert!(first.columns[0].is_pk);

    // A fragment that is not one read statement is refused before it is sent anywhere.
    let refused = page("1=1; DELETE FROM person", false).unwrap_err();
    assert_eq!(refused.kind, DbFailureKind::Query);
    assert_eq!(rig.rows(conn, "SELECT count(*) FROM person"), vec![vec![Value::Int(2)]]);
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
    let results = rig.run(
        conn,
        DbSessionId::generate(),
        &["DELETE FROM person"],
        opts,
    );
    let refused = results[0].as_ref().unwrap_err();
    assert_eq!(refused.kind, DbFailureKind::ReadOnly);
    assert!(refused.message.contains("read-only"), "{}", refused.message);
    assert_eq!(rig.rows(conn, "SELECT count(*) FROM person"), vec![vec![Value::Int(2)]]);

    // A read-only *connection* makes every run read-only, whatever the message says.
    let ro = rig.save(None, rig.sqlite_config("people", true), SecretEdit::Keep);
    let ro = ro.iter().find(|c| c.config.read_only).unwrap().id;
    let results = rig.run(
        ro,
        DbSessionId::generate(),
        &["INSERT INTO person (name) VALUES ('x')"],
        DbRunOptions::default(),
    );
    assert_eq!(results[0].as_ref().unwrap_err().kind, DbFailureKind::ReadOnly);
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
    assert_eq!(rig.rows(conn, "SELECT count(*) FROM person"), vec![vec![Value::Int(2)]]);

    assert_eq!(apply(vec![insert(10, "linus"), insert(11, "ken")]), Ok(2));
    assert_eq!(rig.rows(conn, "SELECT count(*) FROM person"), vec![vec![Value::Int(4)]]);
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
    assert_eq!(results[0].as_ref().unwrap_err().kind, DbFailureKind::Cancelled);
    assert!(stopped_at.elapsed() < Duration::from_secs(10));

    // The session survives it.
    let again = rig.run(conn, session, &["SELECT 1"], DbRunOptions::default());
    assert!(again[0].is_ok());
    assert_eq!(rig.db.inner.runs.in_flight(), 0);
}

#[test]
fn a_password_that_cannot_be_opened_is_a_prompt_and_the_answer_connects() {
    let rig = Rig::new();
    let conn = rig
        .save(
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
    assert_eq!(rig.db.inner.secrets.state(rig.project, conn), PasswordState::Session);
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
        Message::DbTested { probe: p, result, .. } if p == probe => Some(result),
        _ => None,
    });
    assert!(!tested.expect("connects").is_empty());

    let path = rig.dir.path().join("fresh.sqlite").to_string_lossy().into_owned();
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
