//! Agent-controlled query panels (T-290): a host-side shared editor opens a SQL tab in the window,
//! its text is adopted without echoing back as an edit, and the SQL panel lives in the centre.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq::state::db::{DbSqlTab, db_sql_key};
use ubiq::state::dock::{PanelKind, Region};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::db::DbEditor;
use ubiq_proto::ids::{DbConnId, DbQueryId, DbSessionId, ProjectId};
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

fn an_editor(text: &str, rev: u64) -> DbEditor {
    DbEditor {
        name: "scratch".to_string(),
        session: DbSessionId::generate(),
        conn: DbConnId::generate(),
        database: None,
        text: text.to_string(),
        rev,
        agent_key: Some("claude-1".to_string()),
        agent_title: Some("Claude".to_string()),
    }
}

// ── The placement ───────────────────────────────────────────────────

/// T-285: the SQL editor is a centre tab. A layout saved while it lived in the bottom dock has it
/// in a region its class now forbids, which `enforce_placement` puts back to `home()`.
#[test]
fn the_sql_editor_belongs_to_the_centre() {
    let sql = PanelKind::DbSql("dbsql:01J0".to_string());
    assert_eq!(sql.home(), Region::Centre);
    assert!(sql.class().allows(Region::Centre));
    for region in [Region::Left, Region::Right, Region::Bottom] {
        assert!(!sql.class().allows(region), "{region:?}");
    }
    assert_eq!(sql.name(), "ubiq.db.sql", "the saved name never changes");
}

// ── The tab's state ─────────────────────────────────────────────────

#[test]
fn an_agent_tab_is_marked_and_mirrors_the_host() {
    let editor = an_editor("select 1", 3);
    let tab = DbSqlTab::from_editor(&editor);
    assert_eq!(tab.session, editor.session);
    assert_eq!(tab.rev, 3);
    assert_eq!(tab.host_text, "select 1");
    let mark = tab.agent.as_ref().expect("an agent controls it");
    assert_eq!(mark.tooltip(), "Controlled by Claude");
    assert_eq!(mark.chip(), "Agent · Claude");
}

/// The echo guard: an edit is due only when the text differs from the host's, and the host's
/// broadcast of an edit this tab sent is not news.
#[test]
fn an_edit_is_not_echoed() {
    let editor = an_editor("select 1", 1);
    let mut tab = DbSqlTab::from_editor(&editor);
    assert!(!tab.edit_due("select 1"), "same as the host");
    assert!(tab.edit_due("select 2"));

    tab.sent_edit("select 2");
    assert!(!tab.edit_due("select 2"), "already sent");

    // The host accepts it and broadcasts: only the revision moves, the editor is left alone.
    let echoed = DbEditor {
        text: "select 2".to_string(),
        rev: 2,
        ..editor.clone()
    };
    assert!(!tab.adopt_editor(&echoed));
    assert_eq!((tab.rev, tab.pending_text.is_none()), (2, true));

    // The agent writes: the text is adopted and is not an edit to send back.
    let written = DbEditor {
        text: "select 3".to_string(),
        rev: 3,
        ..editor.clone()
    };
    assert!(tab.adopt_editor(&written));
    assert_eq!(tab.pending_text.as_deref(), Some("select 3"));
    assert!(!tab.edit_due("select 3"));

    // A tab the user owns sends nothing.
    let mut own = DbSqlTab::new(DbConnId::generate(), None);
    own.host_text = String::new();
    assert!(!own.edit_due("select 9"));
}

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

    /// `(has the tab, its text, its rev, controlled by)` for a session.
    fn tab(
        &self,
        session: DbSessionId,
        cx: &mut TestAppContext,
    ) -> Option<(String, u64, Option<String>)> {
        self.state.read_with(cx, |state, cx| {
            let open = state.open_project(cx)?;
            let tab = open.db.sql_tab(&db_sql_key(session))?;
            Some((
                tab.text.clone(),
                tab.rev,
                tab.agent.as_ref().map(|a| a.title.clone()),
            ))
        })
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

#[gpui::test]
fn an_editor_changed_opens_the_tab(cx: &mut TestAppContext) {
    let fx = Fixture::open(cx);
    let editor = an_editor("select 1", 1);
    let session = editor.session;
    assert!(fx.tab(session, cx).is_none());

    fx.deliver(
        Message::DbEditorChanged {
            project_id: fx.project,
            editor: Box::new(editor.clone()),
            reveal: true,
        },
        cx,
    );
    assert_eq!(
        fx.tab(session, cx),
        Some(("select 1".to_string(), 1, Some("Claude".to_string())))
    );

    // A later change reuses the tab and takes the new text.
    fx.deliver(
        Message::DbEditorChanged {
            project_id: fx.project,
            editor: Box::new(DbEditor {
                text: "select 2".to_string(),
                rev: 2,
                ..editor
            }),
            reveal: false,
        },
        cx,
    );
    assert_eq!(
        fx.tab(session, cx).map(|t| (t.0, t.1)),
        Some(("select 2".to_string(), 2))
    );
    let count = fx.state.read_with(cx, |state, cx| {
        state.open_project(cx).map_or(0, |open| open.db.sqls.len())
    });
    assert_eq!(count, 1);
}

#[gpui::test]
fn an_agent_run_shows_in_progress(cx: &mut TestAppContext) {
    let fx = Fixture::open(cx);
    let editor = an_editor("select 1", 1);
    let session = editor.session;
    fx.deliver(
        Message::DbEditorChanged {
            project_id: fx.project,
            editor: Box::new(editor),
            reveal: false,
        },
        cx,
    );
    let query = DbQueryId::generate();
    fx.deliver(
        Message::DbAgentRun {
            project_id: fx.project,
            session,
            query,
            statements: vec!["select 1".to_string()],
        },
        cx,
    );
    let running = fx.state.read_with(cx, |state, cx| {
        let open = state.open_project(cx).unwrap();
        open.db.sql_tab(&db_sql_key(session)).map(|t| t.query)
    });
    assert_eq!(running, Some(Some(query)));
    // Nothing was sent back to the host by merely hearing about it.
    assert!(
        !fx.said()
            .iter()
            .any(|m| matches!(m, Message::DbEditorEdit { .. }))
    );
}

/// T-292: only one edit is in flight. A second burst of typing waits for the first one's echo, then
/// goes out against the new revision, so the host never refuses it as stale and the editor is never
/// overwritten with the host's older text.
#[test]
fn two_quick_edits_both_land() {
    let editor = an_editor("select 1", 1);
    let mut tab = DbSqlTab::from_editor(&editor);

    tab.sent_edit("select 2");
    assert!(!tab.edit_due("select 3"), "the first edit is still in flight");

    // The host accepts the first: only the revision moves, and the second is now due at rev 2.
    let echoed = DbEditor {
        text: "select 2".to_string(),
        rev: 2,
        ..editor.clone()
    };
    assert!(!tab.adopt_editor(&echoed), "the user's own edit is not news");
    assert!(tab.pending_text.is_none(), "no clobber");
    assert!(tab.edit_due("select 3"));
    tab.sent_edit("select 3");
    assert_eq!(tab.rev, 2);

    let echoed = DbEditor {
        text: "select 3".to_string(),
        rev: 3,
        ..editor.clone()
    };
    assert!(!tab.adopt_editor(&echoed));
    assert_eq!((tab.rev, tab.host_text.as_str()), (3, "select 3"));
    assert!(!tab.edit_due("select 3"));

    // An agent that wrote in between still wins: the refusal carries its text.
    tab.sent_edit("select 4");
    let refused = DbEditor {
        text: "select agent".to_string(),
        rev: 4,
        ..editor
    };
    assert!(tab.adopt_editor(&refused));
    assert_eq!(tab.pending_text.as_deref(), Some("select agent"));
}
