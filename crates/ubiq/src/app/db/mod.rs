//! The database explorer's wire: what a gesture asks of the host, and where each reply lands.
//!
//! **Every request has one sender here**, so the shapes below are the only place a `Db*` message
//! is built; the packages that draw the explorer, a table tab and a SQL tab call these and never
//! reach `self.bus` for the family themselves. `receive_db` is the one place the replies arrive
//! ([`AppState::receive_db`], called from `app/wire.rs`): the list, the connection states and the
//! Test answers are filed on [`DbState`] here, and each of the others is handed to the handler its
//! package owns — `explorer.rs` the tree, `table.rs` the pages and edits, `sql.rs` the statements,
//! `settings.rs` the file creation.
//!
//! **A reply for a tab the user has closed is discarded by id** — the handlers look the session
//! up and do nothing when it is gone.

use super::*;
use crate::state::db::{DbState, db_sql_from_key, db_table_from_key};
use ubiq_proto::db::{
    ConnectionConfig, DbFailure, DbNode, DbOutcome, DbRun, DbRunOptions, RowEdit, SecretEdit,
    TableRef,
};
use ubiq_proto::ids::{DbConnId, DbProbeId, DbQueryId, DbSessionId};
use ubiq_proto::messages::Secret;

mod explorer;
mod settings;
mod sql;
mod table;

/// One page of a table: the structured request, never SQL. The host renders it with the same
/// `ubiq_db::edit` functions the preview used.
pub struct DbPageRequest {
    pub conn: DbConnId,
    pub session: DbSessionId,
    pub table: TableRef,
    /// The WHERE fragment; empty for none.
    pub filter: String,
    /// The ORDER BY fragment; empty for none.
    pub order_by: String,
    pub limit: u32,
    pub offset: u64,
    /// Also ask for the exact `count(*)`, which the first page does.
    pub count: bool,
    pub read_only: bool,
}

/// One SQL run: the statements already split, and how to limit them.
pub struct DbQueryRequest {
    pub conn: DbConnId,
    pub session: DbSessionId,
    pub database: Option<String>,
    pub statements: Vec<String>,
    pub run: DbRun,
    pub opts: DbRunOptions,
}

/// One `DbQueryResult`: the answer to one statement of a run.
pub struct DbQueryReply {
    pub session: DbSessionId,
    pub query: DbQueryId,
    /// The statement's place in the run's `statements`.
    pub index: u32,
    /// The run's final reply; a failure ends the run early and is `last`.
    pub last: bool,
    pub result: Result<Box<DbOutcome>, DbFailure>,
    pub elapsed_ms: u64,
}

impl AppState {
    /// The databases of the project on screen.
    pub fn db(&self, cx: &App) -> Option<&DbState> {
        self.open_project(cx).map(|open| &open.db)
    }

    pub fn db_mut(&mut self, cx: &App) -> Option<&mut DbState> {
        let id = self.project(cx)?;
        self.projects.get_mut(&id).map(|open| &mut open.db)
    }

    // ── Arrival and settings ────────────────────────────────────────

    /// What this project's connections are. Answered with `DbConnectionsListed`, which
    /// `receive_db` files on [`DbState::accept`].
    pub fn ask_db_connections(&mut self, project: ProjectId) {
        self.bus.send(Message::DbConnections {
            project_id: project,
        });
    }

    /// [`Self::ask_db_connections`], as DB mode's `on_enter` (`D184`) — once, until the host has
    /// answered.
    pub(crate) fn ask_db_connections_on_arrival(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        if self
            .projects
            .get(&project)
            .is_some_and(|open| !open.db.loaded)
        {
            self.ask_db_connections(project);
        }
    }

    /// The same ask, as the Databases settings section's `on_show`: the section works in a project
    /// that has not opted into the mode, so nothing else will have asked.
    pub(crate) fn on_show_project_db(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.editing_project() else {
            return;
        };
        if self
            .projects
            .get(&project)
            .is_some_and(|open| !open.db.loaded)
        {
            self.ask_db_connections(project);
        }
        cx.notify();
    }

    /// Raise the project settings dialog open on the Databases section — `open_kb_settings`'s twin.
    pub fn open_db_settings(&mut self, cx: &mut Context<Self>) {
        self.open_edit_project(cx);
        let Some(settings) = self.workbench.project_settings.as_mut() else {
            return;
        };
        settings.nav = ProjectNav(ext_ids::PROJECT_DB);
        self.on_show_project_db(cx);
        cx.notify();
    }

    /// What the explorer's filter field now holds.
    pub(super) fn retype_db_filter(&mut self, value: String, cx: &mut Context<Self>) {
        if let Some(db) = self.db_mut(cx) {
            db.tree.filter = value;
            cx.notify();
        }
    }

    // ── Requests ────────────────────────────────────────────────────

    /// Create (`id` absent) or edit a connection. The password rides `password`; `config.password`
    /// is cleared here so nothing else can put one on the wire by accident. Answered with the list.
    pub fn save_db_connection(
        &mut self,
        project: ProjectId,
        id: Option<DbConnId>,
        mut config: ConnectionConfig,
        password: SecretEdit,
        remember: bool,
        agent: ubiq_proto::db::DbAgentSettings,
    ) {
        config.password = None;
        self.bus.send(Message::SaveDbConnection {
            project_id: project,
            id,
            config: Box::new(config),
            password,
            remember,
            agent: Some(agent),
        });
    }

    /// Remove a connection; its sessions close here as well, so no tab outlives the connection it
    /// was reading. Answered with the list.
    pub fn delete_db_connection(&mut self, project: ProjectId, id: DbConnId) {
        self.close_db_tabs_of(project, id);
        self.bus.send(Message::DeleteDbConnection {
            project_id: project,
            id,
        });
    }

    /// Try a configuration without saving it, and answer with the probe to look the result up by
    /// in `DbState::probes`. `id` with `Keep` means the saved password, used host-side.
    pub fn test_db_connection(
        &mut self,
        project: ProjectId,
        id: Option<DbConnId>,
        mut config: ConnectionConfig,
        password: SecretEdit,
    ) -> DbProbeId {
        config.password = None;
        let probe = DbProbeId::generate();
        self.bus.send(Message::TestDbConnection {
            project_id: project,
            probe,
            id,
            config: Box::new(config),
            password,
        });
        probe
    }

    /// The password a `NeedsPassword` connection asked for. The prompt goes down at once; the
    /// answer is a `DbConnectionState`.
    pub fn send_db_password(
        &mut self,
        project: ProjectId,
        conn: DbConnId,
        password: String,
        remember: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(open) = self.projects.get_mut(&project) {
            open.db.password_prompt = None;
        }
        self.bus.send(Message::DbPassword {
            project_id: project,
            conn,
            password: Secret::new(password),
            remember,
        });
        cx.notify();
    }

    /// The children of one node of a connection's tree. Answered with `DbTreeListing`.
    pub fn ask_db_tree(&mut self, project: ProjectId, conn: DbConnId, node: DbNode) {
        self.bus.send(Message::DbTree {
            project_id: project,
            conn,
            node,
        });
    }

    /// One page of a table. Answers the run's id: the tab keeps it, and a reply naming any other
    /// is stale. Answered with `DbTablePageResult`.
    pub fn ask_db_table_page(&mut self, project: ProjectId, request: DbPageRequest) -> DbQueryId {
        let query = DbQueryId::generate();
        self.bus.send(Message::DbTablePage {
            project_id: project,
            conn: request.conn,
            session: request.session,
            query,
            table: Box::new(request.table),
            filter: request.filter,
            order_by: request.order_by,
            limit: request.limit,
            offset: request.offset,
            count: request.count,
            read_only: request.read_only,
        });
        query
    }

    /// Run SQL. Answers the run's id, which `cancel_db_query` stops it by. Answered with one
    /// `DbQueryResult` per statement.
    pub fn run_db_query(&mut self, project: ProjectId, request: DbQueryRequest) -> DbQueryId {
        let query = DbQueryId::generate();
        self.bus.send(Message::DbQuery {
            project_id: project,
            conn: request.conn,
            session: request.session,
            query,
            database: request.database,
            statements: request.statements,
            run: request.run,
            opts: request.opts,
        });
        query
    }

    /// Apply a batch of row edits in one transaction. Answers the run's id; answered with
    /// `DbEditsApplied`.
    pub fn apply_db_edits(
        &mut self,
        project: ProjectId,
        conn: DbConnId,
        session: DbSessionId,
        table: TableRef,
        edits: Vec<RowEdit>,
    ) -> DbQueryId {
        let query = DbQueryId::generate();
        self.bus.send(Message::DbApplyEdits {
            project_id: project,
            conn,
            session,
            query,
            table: Box::new(table),
            edits,
        });
        query
    }

    /// Stop a running statement. Nothing answers: the running reply ends `Cancelled`.
    pub fn cancel_db_query(&mut self, project: ProjectId, session: DbSessionId, query: DbQueryId) {
        self.bus.send(Message::DbCancel {
            project_id: project,
            session,
            query,
        });
    }

    /// Close a tab's session on the host. Nothing answers.
    pub fn close_db_session(&mut self, project: ProjectId, session: DbSessionId) {
        self.bus.send(Message::DbCloseSession {
            project_id: project,
            session,
        });
    }

    /// Drop every session of a connection. Answered with `DbConnectionState`.
    pub fn disconnect_db(&mut self, project: ProjectId, conn: DbConnId) {
        self.bus.send(Message::DbDisconnect {
            project_id: project,
            conn,
        });
    }

    /// Create an empty SQLite database at a host path the host-browse picker chose. Answered with
    /// `DbFileCreated` or `DbFileError`.
    pub fn create_db_file(&mut self, project: ProjectId, path: String) {
        self.bus.send(Message::CreateDbFile {
            project_id: project,
            path,
        });
    }

    // ── Tabs ────────────────────────────────────────────────────────

    /// A tab's × (or a removal by the dock): its state goes, its panel with it, and the host is
    /// told to drop the session. Called from the panel's `on_removed`, like `closed_kb_panel`.
    pub fn closed_db_panel(&mut self, kind: &PanelKind, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(key) = kind.db_key() else {
            return;
        };
        let session = self.projects.get_mut(&project).and_then(|open| {
            match (db_table_from_key(key), db_sql_from_key(key)) {
                (Some(_), _) => open
                    .db
                    .tables
                    .iter()
                    .position(|tab| tab.key == key)
                    .map(|at| open.db.tables.remove(at).session),
                (_, Some(session)) => open
                    .db
                    .sqls
                    .iter()
                    .position(|tab| tab.session == session)
                    .map(|at| open.db.sqls.remove(at).session),
                _ => None,
            }
        });
        if let Some(session) = session {
            self.close_db_session(project, session);
            self.drop_db_sql_draft(project, session);
        }
        self.panels.remove(kind);
        cx.notify();
    }

    /// Close every tab of one connection — it was removed or edited out from under them.
    fn close_db_tabs_of(&mut self, project: ProjectId, conn: DbConnId) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let tables: Vec<_> = open
            .db
            .tables
            .iter()
            .filter(|tab| tab.conn == conn)
            .map(|tab| (tab.key.clone(), tab.session))
            .collect();
        let sqls: Vec<_> = open
            .db
            .sqls
            .iter()
            .filter(|tab| tab.conn == conn)
            .map(|tab| (tab.key(), tab.session))
            .collect();
        open.db.tables.retain(|tab| tab.conn != conn);
        open.db.sqls.retain(|tab| tab.conn != conn);
        for (key, session) in tables {
            self.pending_panels
                .push(PanelEdit::Close(PanelKind::DbTable(key)));
            self.close_db_session(project, session);
        }
        for (key, session) in sqls {
            self.pending_panels
                .push(PanelEdit::Close(PanelKind::DbSql(key)));
            self.close_db_session(project, session);
        }
    }

    /// Build the widgets the tabs and the connection form queued, in `render`, where there is a
    /// `Window`. Each package builds its own.
    pub(super) fn build_db_widgets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.build_db_table_widgets(window, cx);
        self.build_db_sql_widgets(window, cx);
        self.build_db_form_widgets(window, cx);
    }

    // ── Replies ─────────────────────────────────────────────────────

    /// The database family's replies.
    ///
    /// Every arm is guarded on the project still being held, the file family's rule: a reply can
    /// land after the window has stopped holding the project it names.
    ///
    /// Answers with the message when it belongs to another family.
    pub(super) fn receive_db(
        &mut self,
        _host: HostRef,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            Message::DbConnectionsListed {
                project_id,
                connections,
                keystore,
            } => {
                let open = self.projects.get_mut(&project_id)?;
                open.db.accept(connections, keystore);
                // A connection the list no longer holds takes its tabs with it.
                let gone: Vec<DbConnId> = open
                    .db
                    .tables
                    .iter()
                    .map(|tab| tab.conn)
                    .chain(open.db.sqls.iter().map(|tab| tab.conn))
                    .filter(|conn| open.db.connection(*conn).is_none())
                    .collect();
                for conn in gone {
                    self.close_db_tabs_of(project_id, conn);
                }
                self.ask_db_editors_once(project_id);
                cx.notify();
            }

            Message::DbEditorsListed {
                project_id,
                editors,
            } => {
                for editor in editors {
                    self.on_db_editor(project_id, editor, false, cx);
                }
            }

            Message::DbEditorChanged {
                project_id,
                editor,
                reveal,
            } => self.on_db_editor(project_id, *editor, reveal, cx),

            Message::DbAgentRun {
                project_id,
                session,
                query,
                statements,
            } => self.on_db_agent_run(project_id, session, query, statements, cx),

            Message::DbConnectionState {
                project_id,
                conn,
                state,
            } => {
                let open = self.projects.get_mut(&project_id)?;
                open.db.set_state(conn, state);
                self.on_db_connection_state(project_id, conn, cx);
                cx.notify();
            }

            Message::DbTested {
                project_id,
                probe,
                result,
            } => {
                let open = self.projects.get_mut(&project_id)?;
                open.db.probes.insert(probe, result);
                cx.notify();
            }

            Message::DbTreeListing {
                project_id,
                conn,
                node,
                result,
            } => self.on_db_tree_listing(project_id, conn, node, result, cx),

            Message::DbTablePageResult {
                project_id,
                session,
                query,
                result,
                elapsed_ms,
            } => self.on_db_table_page(project_id, session, query, result, elapsed_ms, cx),

            Message::DbEditsApplied {
                project_id,
                session,
                query,
                result,
            } => self.on_db_edits_applied(project_id, session, query, result, cx),

            Message::DbQueryResult {
                project_id,
                session,
                query,
                index,
                last,
                result,
                elapsed_ms,
            } => {
                let reply = DbQueryReply {
                    session,
                    query,
                    index,
                    last,
                    result,
                    elapsed_ms,
                };
                self.on_db_query_result(project_id, reply, cx)
            }

            Message::DbFileCreated { project_id, path } => {
                self.on_db_file_created(project_id, path, cx)
            }

            Message::DbFileError {
                project_id,
                path,
                message,
            } => self.on_db_file_error(project_id, path, message, cx),

            Message::DbDbmlReady {
                project_id,
                request,
                result,
                ..
            } => self.on_db_dbml_ready(project_id, request, result, cx),

            other => return Some(other),
        }
        None
    }
}
