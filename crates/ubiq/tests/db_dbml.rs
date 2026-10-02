//! The DB explorer's DBML export (T-293): the scope each node asks for, the menu entries that
//! carry it, and where the host's answer goes.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq::state::db::tree::{DbAction, DbMenuRow, Load, Node, NodeKind, db_menu_entries};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::db::{DbFailure, DbFailureKind, DbObject, ObjectKind, StructureScope};
use ubiq_proto::ids::{DbConnId, DbExportId, ProjectId};
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};


// ── The window ──────────────────────────────────────────────────────

struct Fixture {
    state: Entity<AppState>,
    _window: WindowHandle<Root>,
    host: bus::HostEnd,
    project: ProjectId,
}

impl Fixture {
    fn open(cx: &mut TestAppContext) -> Self {
        let snapshot = a_project();
        let project = snapshot.record.id;
        let (hub, host) = bus::hub();
        cx.update(|cx| {
            gpui_component::init(cx);
            ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
            BusHub::install(hub, cx);
            WindowRegistry::install(cx);
            cx.global_mut::<WindowRegistry>().apply(snapshot);
        });
        let held: Rc<RefCell<Option<Entity<AppState>>>> = Rc::default();
        let taken = held.clone();
        let window = cx.add_window(move |window, cx| {
            let state = cx.new(|cx| AppState::for_project(Some(project), 'A', window, cx));
            *taken.borrow_mut() = Some(state.clone());
            Root::new(state, window, cx)
        });
        cx.run_until_parked();
        let state = held
            .borrow_mut()
            .take()
            .expect("the window built its state");
        Self {
            state,
            _window: window,
            host,
            project,
        }
    }

    fn deliver(&self, message: Message, cx: &mut TestAppContext) {
        self.host.send(To::Everyone, message);
        cx.run_until_parked();
    }

    fn said(&self) -> Vec<Message> {
        let mut said = Vec::new();
        while let Ok(event) = self.host.recv_timeout(Duration::from_millis(300)) {
            if let FromClient::Said { message, .. } = event {
                said.push(message);
            }
        }
        said
    }

}

fn a_project() -> ProjectSnapshot {
    ProjectSnapshot {
        record: ProjectRecord {
            id: ProjectId::generate(),
            name: "ubiq".to_string(),
            path: "/tmp/ubiq".to_string(),
            colour: 0,
            custom_colour: None,
            storage: Default::default(),
            temporary: false,
            created_at: Utc::now(),
            last_opened_at: None,
            search_excludes: Vec::new(),
            index: None,
            mission_term: None,
            tools: Vec::new(),
            managed_repos: Vec::new(),
            lanes: Vec::new(),
            runs_on: None,
            initials: String::new(),
            definitions_use_global: true,
            definitions_allowed: Vec::new(),
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

// ── The tree ────────────────────────────────────────────────────────

fn node(id: &str, conn: DbConnId, kind: NodeKind, label: &str, children: Vec<Node>) -> Node {
    Node {
        id: id.to_string(),
        conn,
        kind,
        label: label.to_string(),
        detail: String::new(),
        expanded: true,
        load: Load::Done,
        children,
    }
}

/// `conn → shop → public → users`, with ids `c`, `c/d`, `c/d/s`, `c/d/s/t`.
fn tree(conn: DbConnId) -> Node {
    let table = DbObject {
        kind: ObjectKind::Table,
        name: "users".into(),
        schema: Some("public".into()),
        database: Some("shop".into()),
        target: None,
        comment: None,
        approx_rows: None,
    };
    let t = node("c/d/s/t", conn, NodeKind::Object(table), "users", vec![]);
    let s = node(
        "c/d/s",
        conn,
        NodeKind::Schema {
            database: "shop".into(),
            schema: "public".into(),
        },
        "public",
        vec![t],
    );
    let d = node("c/d", conn, NodeKind::Database("shop".into()), "shop", vec![s]);
    node("c", conn, NodeKind::Connection, "prod", vec![d])
}

impl Fixture {
    fn with_tree(&self, conn: DbConnId, cx: &mut TestAppContext) {
        self.state.update(cx, |state, cx| {
            let db = state.db_mut(cx).expect("the project is open");
            db.tree.roots = vec![tree(conn)];
        });
    }

    fn export(&self, conn: DbConnId, id: &str, sink: ubiq::state::db::DbmlSink, cx: &mut TestAppContext) {
        self.state
            .update(cx, |state, cx| state.export_dbml(conn, id, sink, cx));
    }
}

#[test]
fn each_node_kind_asks_for_its_own_scope() {
    let conn = DbConnId::generate();
    let root = tree(conn);
    let scope = |id: &str| root.find(id).expect(id).dbml_scope();
    assert_eq!(scope("c"), Some((None, StructureScope::default())));
    assert_eq!(scope("c/d"), Some((Some("shop".into()), StructureScope::default())));
    assert_eq!(
        scope("c/d/s"),
        Some((
            Some("shop".into()),
            StructureScope {
                schema: Some("public".into()),
                tables: vec![]
            }
        ))
    );
    assert_eq!(
        scope("c/d/s/t"),
        Some((
            Some("shop".into()),
            StructureScope {
                schema: Some("public".into()),
                tables: vec!["users".into()]
            }
        ))
    );
    let column = node("x", conn, NodeKind::Column { pk: false }, "id", vec![]);
    assert_eq!(column.dbml_scope(), None);
}

#[test]
fn the_menu_offers_both_dbml_entries_on_the_right_rows() {
    for row in [
        DbMenuRow::Connection { connected: true },
        DbMenuRow::Container,
        DbMenuRow::Relation,
    ] {
        let entries = db_menu_entries(row);
        assert!(entries.contains(&DbAction::CopyDbml), "{row:?}");
        assert!(entries.contains(&DbAction::OpenDbml), "{row:?}");
    }
    for row in [DbMenuRow::Connection { connected: false }, DbMenuRow::Column] {
        assert!(!db_menu_entries(row).contains(&DbAction::CopyDbml), "{row:?}");
    }
}

// ── The window ──────────────────────────────────────────────────────

#[gpui::test]
fn the_export_is_sent_with_the_nodes_scope(cx: &mut TestAppContext) {
    let fx = Fixture::open(cx);
    let conn = DbConnId::generate();
    fx.with_tree(conn, cx);
    fx.export(conn, "c/d/s/t", ubiq::state::db::DbmlSink::Copy, cx);
    let sent = fx.said().into_iter().find_map(|m| match m {
        Message::DbExportDbml {
            conn: c,
            database,
            scope,
            ..
        } => Some((c, database, scope)),
        _ => None,
    });
    assert_eq!(
        sent,
        Some((
            conn,
            Some("shop".to_string()),
            StructureScope {
                schema: Some("public".into()),
                tables: vec!["users".into()]
            }
        ))
    );
}

fn ready(fx: &Fixture, conn: DbConnId, request: DbExportId, result: Result<String, DbFailure>) -> Message {
    Message::DbDbmlReady {
        project_id: fx.project,
        conn,
        request,
        result,
    }
}

fn request_of(fx: &Fixture) -> DbExportId {
    fx.said()
        .into_iter()
        .find_map(|m| match m {
            Message::DbExportDbml { request, .. } => Some(request),
            _ => None,
        })
        .expect("an export was sent")
}

#[gpui::test]
fn a_ready_reply_copies(cx: &mut TestAppContext) {
    let fx = Fixture::open(cx);
    let conn = DbConnId::generate();
    fx.with_tree(conn, cx);
    fx.export(conn, "c/d", ubiq::state::db::DbmlSink::Copy, cx);
    let request = request_of(&fx);
    fx.deliver(ready(&fx, conn, request, Ok("Table users {}".into())), cx);
    let text = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(text.as_deref(), Some("Table users {}"));
}

#[gpui::test]
fn a_ready_reply_opens_an_untitled_file(cx: &mut TestAppContext) {
    let fx = Fixture::open(cx);
    let conn = DbConnId::generate();
    fx.with_tree(conn, cx);
    for _ in 0..2 {
        fx.export(conn, "c/d/s/t", ubiq::state::db::DbmlSink::Open, cx);
    }
    let requests: Vec<DbExportId> = fx
        .said()
        .into_iter()
        .filter_map(|m| match m {
            Message::DbExportDbml { request, .. } => Some(request),
            _ => None,
        })
        .collect();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        fx.deliver(ready(&fx, conn, *request, Ok("Table users {}".into())), cx);
    }
    // Answered once: a repeat is dropped.
    fx.deliver(ready(&fx, conn, requests[0], Ok("again".into())), cx);
    let names: Vec<(String, bool)> = fx.state.read_with(cx, |state, cx| {
        state
            .open_project(cx)
            .map(|open| {
                open.editor
                    .open
                    .iter()
                    .map(|f| (f.path.clone(), f.untitled))
                    .collect()
            })
            .unwrap_or_default()
    });
    assert_eq!(
        names,
        vec![("users.dbml".to_string(), true), ("users-2.dbml".to_string(), true)]
    );
}

#[gpui::test]
fn a_failed_reply_raises_a_notification(cx: &mut TestAppContext) {
    let fx = Fixture::open(cx);
    let conn = DbConnId::generate();
    fx.with_tree(conn, cx);
    fx.export(conn, "c", ubiq::state::db::DbmlSink::Open, cx);
    let request = request_of(&fx);
    let failure = DbFailure::new(DbFailureKind::Query, "no such schema");
    fx.deliver(ready(&fx, conn, request, Err(failure)), cx);
    let raised = fx.said().into_iter().any(|m| {
        matches!(m, Message::RaiseNotification { request } if request.text.contains("no such schema"))
    });
    assert!(raised);
    let opened = fx
        .state
        .read_with(cx, |state, cx| state.open_project(cx).map_or(0, |o| o.editor.open.len()));
    assert_eq!(opened, 0);
}
