//! The explorer's handlers: a tree listing arriving, a row pressed, a menu raised and picked, the
//! keys, and the password prompt a `NeedsPassword` connection raises.
//!
//! Every request goes out through the helpers in `super`; what this file adds is the gesture that
//! decides which one. A node is fetched when it is first opened ([`DbTreeState::toggle`]), never
//! because the filter asked — the filter reads what is already loaded.

use super::*;
use crate::state::MenuId;
use crate::state::db::tree::{DbAction, DbKey, DbMenu, DbTreeState, NodeKind, Toggled};
use gpui::ClipboardItem;
use ubiq_proto::db::{DbConnState, DbKind, DbListing};

impl AppState {
    /// A `DbTreeListing`: file `result` under `node` of `conn`'s tree, or the failure on its row.
    pub(super) fn on_db_tree_listing(
        &mut self,
        project: ProjectId,
        conn: DbConnId,
        node: DbNode,
        result: Result<DbListing, DbFailure>,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(kind) = open.db.connection(conn).map(|c| c.config.kind) else {
            return;
        };
        open.db.tree.apply(conn, kind, &node, result);
        cx.notify();
    }

    /// A `DbConnectionState`, after `receive_db` filed it: a connection that has just come up
    /// answers the open root that was waiting on it — the password was typed, and the listing that
    /// failed for want of it is asked again.
    pub(super) fn on_db_connection_state(
        &mut self,
        project: ProjectId,
        conn: DbConnId,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        if !matches!(open.db.state_of(conn), DbConnState::Connected { .. }) {
            return;
        }
        let Some(kind) = open.db.connection(conn).map(|c| c.config.kind) else {
            return;
        };
        if let Some(node) = open.db.tree.retry_failed(conn, kind) {
            self.ask_db_tree(project, conn, node);
        }
        cx.notify();
    }

    // ── Opening and fetching ────────────────────────────────────────

    /// Run a tree mutation that may need a `DbTree` sent, and send it.
    fn with_db_tree(
        &mut self,
        conn: DbConnId,
        cx: &mut Context<Self>,
        act: impl FnOnce(&mut DbTreeState, DbKind) -> Toggled,
    ) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(db) = self.db_mut(cx) else {
            return;
        };
        let Some(kind) = db.connection(conn).map(|c| c.config.kind) else {
            return;
        };
        if let Toggled::Ask(node) = act(&mut db.tree, kind) {
            self.ask_db_tree(project, conn, node);
        }
        cx.notify();
    }

    /// Open or close a node; the twisty, and the row for a node that has nothing else to do.
    pub fn toggle_db_node(&mut self, conn: DbConnId, id: String, cx: &mut Context<Self>) {
        self.with_db_tree(conn, cx, |tree, kind| tree.toggle(conn, &id, kind));
    }

    /// Open a connection's root and ask for its databases — which is also what dials it.
    pub fn connect_db(&mut self, conn: DbConnId, cx: &mut Context<Self>) {
        self.with_db_tree(conn, cx, |tree, kind| tree.connect(conn, kind));
    }

    /// A press on a row. It selects; a relation opens its table on the second press and leaves its
    /// columns to the twisty, and anything else opens or closes on the first.
    pub fn click_db_row(
        &mut self,
        conn: DbConnId,
        id: String,
        clicks: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(db) = self.db_mut(cx) else {
            return;
        };
        db.tree.selected = Some(id.clone());
        let Some(node) = db.tree.node(conn, &id) else {
            return;
        };
        let (relation, expandable) = (node.table_ref().is_some(), node.expandable());
        self.focus_db_tree(window, cx);
        match relation {
            true if clicks >= 2 => self.open_db_node_table(conn, &id, cx),
            true => {}
            false if expandable && clicks < 2 => self.toggle_db_node(conn, id, cx),
            false => {}
        }
        cx.notify();
    }

    /// Open the table or view a tree node stands for, and tell its tab what the tree knows that
    /// the tab does not: the estimated row count, and whether it is a table at all (a view or a
    /// synonym is forced read-only).
    fn open_db_node_table(&mut self, conn: DbConnId, id: &str, cx: &mut Context<Self>) {
        let Some((table, rows, editable)) = self
            .db(cx)
            .and_then(|db| db.tree.node(conn, id))
            .and_then(|node| match &node.kind {
                NodeKind::Object(obj) if obj.kind.is_relation() => {
                    Some((obj.table_ref(), obj.approx_rows, obj.kind.is_editable()))
                }
                _ => None,
            })
        else {
            return;
        };
        self.open_db_table(conn, table.clone(), cx);
        self.set_db_table_facts(conn, &table, rows, editable, cx);
    }

    // ── The right-click menu ────────────────────────────────────────

    /// Raise the menu at the pointer. A row with nothing to offer (a group, a note) raises none.
    pub fn open_db_menu(
        &mut self,
        conn: DbConnId,
        id: String,
        at: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        let Some(db) = self.db_mut(cx) else {
            return;
        };
        let state = db.state_of(conn).clone();
        let Some(row) = db.tree.menu_row(conn, &id, &state) else {
            return;
        };
        db.tree.selected = Some(id.clone());
        db.menu_epoch = db.menu_epoch.wrapping_add(1);
        db.menu = Some(DbMenu {
            epoch: db.menu_epoch,
            x: at.0,
            y: at.1,
            conn,
            node: id,
            row,
        });
        self.workbench.open_menu = Some(MenuId::Db);
        cx.notify();
    }

    /// Take the menu down with the window's own `close_menu`, which peels every menu at once.
    pub(crate) fn drop_db_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(db) = self.db_mut(cx) {
            db.menu = None;
        }
    }

    /// The menu's own outside click, carrying the menu it was drawn for: a dismiss for a menu
    /// already replaced by the right-click that raised the next one does nothing.
    pub fn dismiss_db_menu(&mut self, epoch: u64, cx: &mut Context<Self>) {
        let gone = match self.db_mut(cx) {
            Some(db) => {
                if db.menu.as_ref().is_some_and(|menu| menu.epoch == epoch) {
                    db.menu = None;
                }
                db.menu.is_none()
            }
            None => false,
        };
        if gone && self.workbench.open_menu == Some(MenuId::Db) {
            self.workbench.open_menu = None;
        }
        cx.notify();
    }

    /// Act on the entry the menu drew at `index` — an index into [`DbMenu::entries`], so the menu
    /// is taken down and read here rather than by whoever drew it.
    pub fn pick_db_action(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(menu) = self.db_mut(cx).and_then(|db| db.menu.take()) else {
            return;
        };
        self.workbench.open_menu = None;
        let Some(action) = menu.entries().get(index).copied() else {
            cx.notify();
            return;
        };
        let conn = menu.conn;
        let Some(node) = self
            .db(cx)
            .and_then(|db| db.tree.node(conn, &menu.node))
            .map(|node| (node.label.clone(), node.database(), node.table_ref()))
        else {
            cx.notify();
            return;
        };
        let (label, database, table) = node;

        match action {
            DbAction::Connect => self.connect_db(conn, cx),
            DbAction::Disconnect => {
                self.disconnect_db(project, conn);
                if let Some(db) = self.db_mut(cx) {
                    db.tree.reset(conn);
                }
            }
            DbAction::NewSql => self.open_db_sql(conn, database, cx),
            // The editor opens with the table's first page as a `SELECT`, in the engine's own
            // dialect — the same builder the table tab pages with.
            DbAction::OpenInSql => {
                let kind = self
                    .db(cx)
                    .and_then(|db| db.connection(conn))
                    .map(|saved| saved.config.kind);
                let text = match (&table, kind) {
                    (Some(table), Some(kind)) => {
                        ubiq_db::edit::select_table(kind, table, None, "", "", 100, 0)
                    }
                    _ => String::new(),
                };
                self.open_db_sql_text(conn, database, text, cx);
            }
            DbAction::Refresh | DbAction::RefreshCounts => {
                let id = menu.node.clone();
                self.with_db_tree(conn, cx, |tree, kind| tree.refresh(conn, &id, kind));
            }
            DbAction::EditConnection => {
                self.open_db_settings(cx);
                self.open_db_conn_form(Some(conn), window, cx);
            }
            // Removing is a question, and the settings section asks it where the list is.
            DbAction::Remove => {
                self.open_db_settings(cx);
                if let Some(db) = self.db_mut(cx) {
                    db.tree.removing = Some(conn);
                }
            }
            DbAction::OpenData => {
                if table.is_some() {
                    self.open_db_node_table(conn, &menu.node, cx);
                }
            }
            DbAction::CopyName => cx.write_to_clipboard(ClipboardItem::new_string(label)),
            DbAction::Separator => {}
        }
        cx.notify();
    }

    // ── The keyboard ────────────────────────────────────────────────

    /// Put the keyboard on the tree.
    pub fn focus_db_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.db(cx).and_then(|db| db.tree.focus.clone()) {
            window.focus(&focus, cx);
        }
    }

    /// Put the keyboard on the filter field.
    pub fn focus_db_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.db_filter.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    /// Answer a key on the tree. `false` hands it back, so the filter field keeps the keys a field
    /// keeps (`left` and `right` mean a caret there).
    pub fn press_db_key(&mut self, key: DbKey, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(db) = self.db_mut(cx) else {
            return false;
        };
        let filtering = !db.tree.filter.trim().is_empty();
        match key {
            DbKey::Up | DbKey::Down => {
                db.tree.step(if key == DbKey::Up { -1 } else { 1 });
                self.reveal_db_cursor(cx);
                true
            }
            DbKey::Right => {
                let Some((conn, id, open)) = self.db_cursor(cx) else {
                    return false;
                };
                let expandable = self
                    .db(cx)
                    .and_then(|db| db.tree.node(conn, &id))
                    .is_some_and(|node| node.expandable());
                if expandable && !open && !filtering {
                    self.toggle_db_node(conn, id, cx);
                } else if let Some(db) = self.db_mut(cx) {
                    db.tree.step(1);
                    self.reveal_db_cursor(cx);
                }
                true
            }
            DbKey::Left => {
                let Some((conn, id, open)) = self.db_cursor(cx) else {
                    return false;
                };
                if open && !filtering {
                    self.toggle_db_node(conn, id, cx);
                } else if let Some(db) = self.db_mut(cx) {
                    match db.tree.parent_of_cursor() {
                        Some(parent) => db.tree.selected = Some(parent),
                        None => return false,
                    }
                    self.reveal_db_cursor(cx);
                }
                true
            }
            DbKey::Enter => {
                // From the filter field with nothing chosen yet, Enter takes the first row the
                // query landed on, so a typed name is one key from its table.
                if db.tree.cursor_index().is_none() {
                    db.tree.step(1);
                }
                let Some((conn, id, _)) = self.db_cursor(cx) else {
                    return false;
                };
                let relation = self
                    .db(cx)
                    .and_then(|db| db.tree.node(conn, &id))
                    .is_some_and(|node| node.table_ref().is_some());
                match relation {
                    true => self.open_db_node_table(conn, &id, cx),
                    false => self.toggle_db_node(conn, id, cx),
                }
                self.reveal_db_cursor(cx);
                true
            }
            DbKey::Dismiss => {
                if !filtering {
                    return false;
                }
                db.tree.filter.clear();
                self.db_filter
                    .update(cx, |state, cx| state.set_value("", window, cx));
                cx.notify();
                true
            }
        }
    }

    /// The connection, node and openness of the row the keyboard is on.
    fn db_cursor(&self, cx: &App) -> Option<(DbConnId, String, bool)> {
        let tree = &self.db(cx)?.tree;
        let id = tree.selected.clone()?;
        let rows = crate::state::db::tree::visible(&tree.roots, &tree.filter);
        let row = rows.iter().find(|row| row.node.id == id)?;
        Some((row.node.conn, id, row.open))
    }

    fn reveal_db_cursor(&mut self, cx: &mut Context<Self>) {
        if let Some(at) = self.db(cx).and_then(|db| db.tree.cursor_index())
            && let Some(db) = self.db(cx)
        {
            db.tree.scroll.scroll_to_item(at);
        }
        cx.notify();
    }

    // ── The password prompt ─────────────────────────────────────────

    /// Send what was typed as the password the connection asked for.
    pub fn submit_db_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(db) = self.db(cx) else {
            return;
        };
        let (Some(prompt), Some(input)) = (db.password_prompt.clone(), db.tree.prompt_input.clone())
        else {
            return;
        };
        let typed = input.read(cx).value().to_string();
        if typed.is_empty() {
            return;
        }
        input.update(cx, |state, cx| state.set_value("", window, cx));
        self.send_db_password(project, prompt.conn, typed, prompt.remember, cx);
    }

    /// Take the prompt down without an answer; the connection stays as it was.
    pub fn cancel_db_password(&mut self, cx: &mut Context<Self>) {
        if let Some(db) = self.db_mut(cx) {
            db.password_prompt = None;
        }
        cx.notify();
    }

    /// Flip *Remember* on the prompt. Does nothing when the keystore cannot seal.
    pub fn toggle_db_password_remember(&mut self, cx: &mut Context<Self>) {
        if let Some(db) = self.db_mut(cx)
            && let Some(prompt) = db.password_prompt.as_mut()
            && !matches!(db.keystore, Some(ubiq_proto::db::DbKeystore::Unavailable(_)))
        {
            prompt.remember = !prompt.remember;
        }
        cx.notify();
    }
}
