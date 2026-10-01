//! A table tab's handlers: opening one, a page arriving, every gesture on the toolbar and the grid,
//! edits applied.
//!
//! Everything is looked up by session or by the tab's key: a reply for a tab the user has closed
//! finds nothing and is dropped. The tab's widgets need a `Window`, so they are built in
//! [`AppState::build_db_table_widgets`], called from `render` (`build_db_widgets`): the grid, the
//! WHERE and ORDER BY fields, the row form's inputs and the JSON dialog. The same pass asks for
//! the first page of a tab that has not asked yet — a tab opened by the explorer, or restored
//! with the layout once the connection list is known — and asks again for one that was waiting on
//! a password.
//!
//! **The pending edits live in the grid** (`ui/db/grid.rs`), which is the only thing that can see
//! a cell as it is drawn; `DbTableTab::pending_rows` mirrors their count after every gesture that
//! changes them ([`AppState::sync_db_table_pending`]), for the tab strip and the toolbar. Apply
//! sends them as `RowEdit`s; the host renders them with the functions the preview used.

use super::*;
use crate::state::db::table::{DbTableTab, EditMode, TableNote, describe_failure};
use crate::state::db::{DbState, db_table_from_key, db_table_key};
use crate::ui::db::table::cell_input::{CellInput, CellInputEvent};
use crate::ui::db::table::grid::{GridEvent, ResultGrid};
use crate::ui::db::table::json::{JsonEditor, JsonEvent, JsonSpec};
use crate::ui::db::table::{FormField, JsonOpen, TableWidgets};
use gpui::{App, Entity, Window};
use gpui_component::input::{InputEvent, InputState};
use ubiq_db::edit::{render_batch, validate_fragment};
use ubiq_db::value::parse_input;
use ubiq_proto::db::{DbConnState, DbEditFailure, DbKind, DbPage};

impl AppState {
    /// Open a table in a tab, or bring forward the one already open. A new tab is a session of
    /// its own; its first page is asked for when its widgets are built.
    pub fn open_db_table(&mut self, conn: DbConnId, table: TableRef, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let key = db_table_key(conn, &table);
        if open.db.table_tab(&key).is_none() {
            open.db.tables.push(DbTableTab::new(conn, table));
            self.pending_panels
                .push(PanelEdit::Open(PanelKind::DbTable(key.clone())));
        }
        self.pending_panels
            .push(PanelEdit::Reveal(PanelKind::DbTable(key)));
        cx.notify();
    }

    /// What the explorer knows of a table that the tab does not: the estimated row count shown
    /// until the exact one arrives, and whether it is a table (`editable`) or a view or synonym.
    pub fn set_db_table_facts(
        &mut self,
        conn: DbConnId,
        table: &TableRef,
        approx_rows: Option<u64>,
        editable: bool,
        cx: &mut Context<Self>,
    ) {
        let key = db_table_key(conn, table);
        if let Some(tab) = self.db_tab_mut(&key, cx) {
            tab.approx_rows = approx_rows;
            tab.editable = editable;
            cx.notify();
        }
    }

    // ── Lookups ─────────────────────────────────────────────────────

    fn db_tab_mut(&mut self, key: &str, cx: &App) -> Option<&mut DbTableTab> {
        let project = self.project(cx)?;
        self.projects.get_mut(&project)?.db.table_tab_mut(key)
    }

    fn db_tab_with_db(&self, key: &str, cx: &App) -> Option<(&DbState, &DbTableTab)> {
        let db = &self.projects.get(&self.project(cx)?)?.db;
        Some((db, db.table_tab(key)?))
    }

    /// The tab's grid, if its widgets are built.
    fn db_grid(&self, key: &str, cx: &App) -> Option<Entity<ResultGrid>> {
        let (_, tab) = self.db_tab_with_db(key, cx)?;
        Some(TableWidgets::of(tab)?.grid.clone())
    }

    /// Whether the tab cannot write: its connection, the tab's kind and its lock.
    fn db_tab_read_only(&self, key: &str, cx: &App) -> bool {
        self.db_tab_with_db(key, cx)
            .is_none_or(|(db, tab)| tab.read_only(db.is_read_only(tab.conn)))
    }

    // ── Replies ─────────────────────────────────────────────────────

    /// A `DbTablePageResult`.
    pub(super) fn on_db_table_page(
        &mut self,
        project: ProjectId,
        session: DbSessionId,
        query: DbQueryId,
        result: Result<Box<DbPage>, DbFailure>,
        elapsed_ms: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self
            .projects
            .get_mut(&project)
            .and_then(|open| open.db.table_by_session(session))
        else {
            return;
        };
        let rows = tab.accept_page(query, elapsed_ms, result);
        let grid = TableWidgets::of(tab).map(|widgets| widgets.grid.clone());
        if let (Some(rows), Some(grid)) = (rows, grid) {
            grid.update(cx, |grid, cx| grid.set_result(rows, cx));
        }
        cx.notify();
    }

    /// A `DbEditsApplied`: the rows changed, or the statement that rolled the batch back.
    pub(super) fn on_db_edits_applied(
        &mut self,
        project: ProjectId,
        session: DbSessionId,
        query: DbQueryId,
        result: Result<u64, DbEditFailure>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self
            .projects
            .get_mut(&project)
            .and_then(|open| open.db.table_by_session(session))
        else {
            return;
        };
        if tab.query != Some(query) {
            return;
        }
        match result {
            Ok(affected) => {
                tab.applied(affected);
                // The reload replaces the page, which clears every pending change.
                let key = tab.key.clone();
                self.db_table_reload(&key, true, cx);
            }
            Err(failure) => {
                tab.applying = false;
                tab.query = None;
                let batch = tab.applying_count;
                let reason = describe_failure(&failure.failure);
                let message = format!(
                    "Statement {} of {batch} failed, rolled back: {reason}\n{}",
                    failure.index + 1,
                    failure.statement
                );
                tab.note = Some(TableNote::error(message));
                // The failing statement is the failing row: the edits are in row order.
                if let Some(grid) = TableWidgets::of(tab).map(|widgets| widgets.grid.clone()) {
                    let row = grid
                        .read(cx)
                        .edits_by_row(cx)
                        .get(failure.index as usize)
                        .map(|(row, _)| *row);
                    if let Some(row) = row {
                        grid.update(cx, |grid, cx| grid.set_row_error(row, Some(reason), cx));
                    }
                }
                cx.notify();
            }
        }
    }

    // ── Asking for pages ────────────────────────────────────────────

    /// Ask for the tab's current page (and, with `recount`, the exact count). A page asked for
    /// supersedes any still on its way.
    pub(crate) fn db_table_reload(&mut self, key: &str, recount: bool, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let request = {
            let Some(open) = self.projects.get(&project) else {
                return;
            };
            let Some(tab) = open.db.table_tab(key) else {
                return;
            };
            DbPageRequest {
                conn: tab.conn,
                session: tab.session,
                table: tab.table.clone(),
                filter: tab.filter.clone(),
                order_by: tab.order_by.clone(),
                // One row more than a page: whether it comes back is "there is a next page".
                limit: tab.page_size + 1,
                offset: tab.offset(),
                count: recount,
                read_only: tab.read_only(open.db.is_read_only(tab.conn)),
            }
        };
        let query = self.ask_db_table_page(project, request);
        if let Some(tab) = self.db_tab_mut(key, cx) {
            tab.begin_load(query, recount);
        }
        cx.notify();
    }

    /// Navigation drops the page's pending edits, so it refuses while there are any.
    fn db_table_may_leave(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.db_tab_mut(key, cx) else {
            return false;
        };
        if !tab.leave_blocked() {
            return true;
        }
        tab.note = Some(TableNote::error("Apply or discard the pending changes first."));
        cx.notify();
        false
    }

    pub(crate) fn db_table_refresh(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.db_table_may_leave(key, cx) {
            self.db_table_reload(key, true, cx);
        }
    }

    pub(crate) fn db_table_turn_page(&mut self, key: &str, forward: bool, cx: &mut Context<Self>) {
        if !self.db_table_may_leave(key, cx) {
            return;
        }
        let Some(tab) = self.db_tab_mut(key, cx) else {
            return;
        };
        if forward && tab.has_more {
            tab.page += 1;
        } else if !forward && tab.page > 0 {
            tab.page -= 1;
        } else {
            return;
        }
        self.db_table_reload(key, false, cx);
    }

    /// The page-size button: the next size of the ladder, from the first page.
    pub(crate) fn db_table_next_page_size(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.db_table_may_leave(key, cx) {
            return;
        }
        let Some(tab) = self.db_tab_mut(key, cx) else {
            return;
        };
        tab.page_size = tab.next_page_size();
        tab.page = 0;
        self.db_table_reload(key, false, cx);
    }

    /// What the WHERE and ORDER BY fields hold, trimmed.
    fn db_table_fragments(&self, key: &str, cx: &App) -> Option<(DbKind, String, String)> {
        let (db, tab) = self.db_tab_with_db(key, cx)?;
        let widgets = TableWidgets::of(tab)?;
        let kind = db.connection(tab.conn)?.config.kind;
        Some((
            kind,
            widgets.where_input.read(cx).value().trim().to_string(),
            widgets.order_input.read(cx).value().trim().to_string(),
        ))
    }

    /// A key in either field: say at once whether the fragments would parse.
    pub(crate) fn db_table_fragments_changed(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some((kind, filter, order_by)) = self.db_table_fragments(key, cx) else {
            return;
        };
        let error = validate_fragment(kind, &filter, &order_by)
            .err()
            .map(|e| format!("{} (line {}, col {})", e.message, e.line, e.column));
        if let Some(tab) = self.db_tab_mut(key, cx) {
            tab.fragment_error = error;
            cx.notify();
        }
    }

    /// Enter in a field, or the Filter button: validate both fragments, then reload page 0.
    pub(crate) fn db_table_apply_fragments(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.db_table_may_leave(key, cx) {
            return;
        }
        let Some((kind, filter, order_by)) = self.db_table_fragments(key, cx) else {
            return;
        };
        let Some(tab) = self.db_tab_mut(key, cx) else {
            return;
        };
        if let Err(e) = validate_fragment(kind, &filter, &order_by) {
            tab.fragment_error = Some(format!("{} (line {}, col {})", e.message, e.line, e.column));
            cx.notify();
            return;
        }
        tab.fragment_error = None;
        tab.filter = filter;
        tab.order_by = order_by;
        tab.page = 0;
        self.db_table_reload(key, true, cx);
    }

    // ── Read-only and the edit mode ─────────────────────────────────

    /// Cells edit in the grid when the tab can edit and the mode is Inline.
    fn sync_db_table_editable(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some((db, tab)) = self.db_tab_with_db(key, cx) else {
            return;
        };
        let editable = tab.grid_editable(db.is_read_only(tab.conn));
        if let Some(grid) = self.db_grid(key, cx) {
            grid.update(cx, |grid, cx| grid.set_editable(editable, window, cx));
        }
    }

    /// The lock. A forced tab cannot be unlocked; locking refuses while edits are pending, since
    /// they would have nowhere to go.
    pub(crate) fn db_table_toggle_lock(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((db, tab)) = self.db_tab_with_db(key, cx) else {
            return;
        };
        if tab.forced(db.is_read_only(tab.conn)) {
            return;
        }
        if !tab.locked && !self.db_table_may_leave(key, cx) {
            return;
        }
        if let Some(tab) = self.db_tab_mut(key, cx) {
            tab.locked = !tab.locked;
            tab.note = None;
        }
        self.sync_db_table_editable(key, window, cx);
        self.populate_db_table_form(key, window, cx);
    }

    pub(crate) fn db_table_set_edit_mode(
        &mut self,
        key: &str,
        mode: EditMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.db_tab_mut(key, cx)
            && tab.edit_mode != mode
        {
            tab.edit_mode = mode;
            self.sync_db_table_editable(key, window, cx);
            cx.notify();
        }
    }

    pub(crate) fn db_table_toggle_sql(&mut self, key: &str, cx: &mut Context<Self>) {
        if let Some(tab) = self.db_tab_mut(key, cx) {
            tab.show_sql = !tab.show_sql;
            cx.notify();
        }
    }

    pub(crate) fn db_table_toggle_inspector(&mut self, key: &str, cx: &mut Context<Self>) {
        if let Some(tab) = self.db_tab_mut(key, cx) {
            tab.show_form = !tab.show_form;
            cx.notify();
        }
    }

    // ── Rows ────────────────────────────────────────────────────────

    /// Copy the grid's pending count onto the tab.
    fn sync_db_table_pending(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(grid) = self.db_grid(key, cx) else {
            return;
        };
        let pending = grid.read(cx).pending_rows(cx);
        if let Some(tab) = self.db_tab_mut(key, cx) {
            tab.pending_rows = pending;
        }
        cx.notify();
    }

    pub(crate) fn db_table_add_row(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(grid) = self.db_grid(key, cx) else {
            return;
        };
        let row = grid.update(cx, |grid, cx| grid.add_row(cx));
        if let Some(tab) = self.db_tab_mut(key, cx) {
            tab.selected = Some(row);
        }
        self.sync_db_table_pending(key, cx);
        self.populate_db_table_form(key, window, cx);
    }

    pub(crate) fn db_table_delete_row(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(grid), Some(row)) = (
            self.db_grid(key, cx),
            self.db_tab_with_db(key, cx).and_then(|(_, tab)| tab.selected),
        ) else {
            return;
        };
        grid.update(cx, |grid, cx| grid.toggle_delete(row, cx));
        self.sync_db_table_pending(key, cx);
        self.populate_db_table_form(key, window, cx);
    }

    pub(crate) fn db_table_discard(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(grid) = self.db_grid(key, cx) else {
            return;
        };
        grid.update(cx, |grid, cx| grid.discard(cx));
        if let Some(tab) = self.db_tab_mut(key, cx) {
            if tab.selected.is_some_and(|row| row >= tab.page_rows) {
                tab.selected = None;
            }
            tab.note = None;
        }
        self.sync_db_table_pending(key, cx);
        self.populate_db_table_form(key, window, cx);
    }

    /// F2: edit the selected cell in place.
    pub(crate) fn db_table_edit_cell(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(grid) = self.db_grid(key, cx) {
            grid.update(cx, |grid, cx| grid.edit_selected(window, cx));
        }
    }

    // ── Apply ───────────────────────────────────────────────────────

    /// Send the pending edits as one transaction.
    pub(crate) fn db_table_apply(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some((db, tab)) = self.db_tab_with_db(key, cx) else {
            return;
        };
        if tab.applying {
            return;
        }
        let (conn, session, table) = (tab.conn, tab.session, tab.table.clone());
        let kind = db.connection(conn).map(|conn| conn.config.kind);
        let read_only = tab.read_only(db.is_read_only(conn));
        let Some(grid) = self.db_grid(key, cx) else {
            return;
        };
        if read_only {
            // Not reachable from the toolbar (Apply is hidden); say it plainly if it is reached.
            if let Some(tab) = self.db_tab_mut(key, cx) {
                tab.note = Some(TableNote::error(
                    "Read-only: nothing was written. This tab is read-only; unlock it to apply changes.",
                ));
            }
            cx.notify();
            return;
        }
        let edits = grid.read(cx).edits(cx);
        if edits.is_empty() {
            return;
        }
        // Rendered here only to refuse early what cannot be rendered; the host renders again.
        if let Some(kind) = kind
            && let Err(error) = render_batch(kind, &table, &edits)
        {
            if let Some(tab) = self.db_tab_mut(key, cx) {
                tab.note = Some(TableNote::error(format!("Cannot apply: {error}")));
            }
            cx.notify();
            return;
        }
        grid.update(cx, |grid, cx| {
            for (row, _) in grid.edits_by_row(cx) {
                grid.set_row_error(row, None, cx);
            }
        });
        let count = edits.len();
        let query = self.apply_db_edits(project, conn, session, table, edits);
        if let Some(tab) = self.db_tab_mut(key, cx) {
            tab.applying = true;
            tab.applying_count = count;
            tab.query = Some(query);
            tab.note = None;
        }
        cx.notify();
    }

    // ── The row form ────────────────────────────────────────────────

    /// Rebuild the form's inputs from the page's columns and fill them from the selected row.
    fn refill_db_table_form(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(columns) = self.db_tab_mut(key, cx).map(|tab| {
            tab.refill = false;
            tab.columns.clone()
        }) else {
            return;
        };
        let fields: Vec<FormField> = columns
            .iter()
            .enumerate()
            .map(|(col, meta)| {
                let input = cx.new(|cx| CellInput::new(meta, false, window, cx));
                let tab_key = key.to_string();
                let subscription = cx.subscribe_in(
                    &input,
                    window,
                    move |this, _, event: &CellInputEvent, window, cx| {
                        this.on_db_field_event(&tab_key, col, *event, window, cx)
                    },
                );
                FormField {
                    input,
                    _subscription: subscription,
                }
            })
            .collect();
        if let Some(tab) = self.db_tab_mut(key, cx) {
            tab.field_errors = vec![None; fields.len()];
            if let Some(widgets) = TableWidgets::of_mut(tab) {
                widgets.fields = fields;
            }
        }
        self.sync_db_table_editable(key, window, cx);
        self.populate_db_table_form(key, window, cx);
    }

    /// Fill the form from the selected row (or empty it) and set which fields can be typed in.
    fn populate_db_table_form(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let read_only = self.db_tab_read_only(key, cx);
        let Some(grid) = self.db_grid(key, cx) else {
            return;
        };
        let Some((_, tab)) = self.db_tab_with_db(key, cx) else {
            return;
        };
        let (row, columns) = (tab.selected, tab.columns.clone());
        let inputs: Vec<Entity<CellInput>> = TableWidgets::of(tab)
            .map(|widgets| widgets.fields.iter().map(|f| f.input.clone()).collect())
            .unwrap_or_default();
        let deleted = row.is_some_and(|row| {
            grid.read(cx).row_mark(row, cx) == crate::state::db::pending::RowMark::Deleted
        });
        for (col, input) in inputs.iter().enumerate() {
            let value = row.and_then(|row| grid.read(cx).cell(row, col, cx).input_text());
            // Read-only: the form is an inspector, nothing in it can be typed.
            let disabled = read_only || deleted || columns.get(col).is_some_and(|c| c.computed);
            input.update(cx, |input, cx| {
                input.load(value.as_deref(), window, cx);
                input.set_disabled(disabled, cx);
            });
        }
        if let Some(tab) = self.db_tab_mut(key, cx) {
            tab.field_errors = vec![None; inputs.len()];
        }
        cx.notify();
    }

    /// Field `col` of the form said something.
    fn on_db_field_event(
        &mut self,
        key: &str,
        col: usize,
        event: CellInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            CellInputEvent::Changed => self.db_table_field_changed(key, col, cx),
            CellInputEvent::OpenJson => self.db_table_open_json_selected(key, col, window, cx),
            _ => {}
        }
    }

    /// Field `col` changed: parse its input (`None` = NULL, `''` is not) against the column, and
    /// make the result the cell's pending value. An input that does not parse leaves the cell as
    /// loaded.
    fn db_table_field_changed(&mut self, key: &str, col: usize, cx: &mut Context<Self>) {
        if self.db_tab_read_only(key, cx) {
            return;
        }
        let Some(grid) = self.db_grid(key, cx) else {
            return;
        };
        let Some((_, tab)) = self.db_tab_with_db(key, cx) else {
            return;
        };
        let (Some(row), Some(meta)) = (tab.selected, tab.columns.get(col).cloned()) else {
            return;
        };
        let Some(field) = TableWidgets::of(tab).and_then(|widgets| widgets.fields.get(col)) else {
            return;
        };
        let input = field.input.read(cx).value(cx);
        let original = grid.read(cx).original(row, col, cx);
        let (value, error) = if input == original.input_text() {
            (original, None)
        } else {
            match parse_input(input.as_deref(), &meta) {
                Ok(value) => (value, None),
                Err(e) => (original, Some(e.message)),
            }
        };
        if let Some(slot) = self
            .db_tab_mut(key, cx)
            .and_then(|tab| tab.field_errors.get_mut(col))
        {
            *slot = error;
        }
        grid.update(cx, |grid, cx| grid.edit_cell(row, col, value, cx));
        self.sync_db_table_pending(key, cx);
    }

    // ── The grid ────────────────────────────────────────────────────

    fn on_db_grid_event(
        &mut self,
        key: &str,
        event: GridEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            GridEvent::Selected(row) => {
                // Another cell of the same row must not wipe what the form holds.
                let Some(tab) = self.db_tab_mut(key, cx) else {
                    return;
                };
                if tab.selected != row {
                    tab.selected = row;
                    self.populate_db_table_form(key, window, cx);
                }
                cx.notify();
            }
            GridEvent::Edited { row, .. } => {
                self.sync_db_table_pending(key, cx);
                if self
                    .db_tab_with_db(key, cx)
                    .is_some_and(|(_, tab)| tab.selected == Some(row))
                {
                    self.populate_db_table_form(key, window, cx);
                }
            }
            GridEvent::OpenJson { row, col } => self.db_table_open_json(key, row, col, window, cx),
        }
    }

    // ── The JSON dialog ─────────────────────────────────────────────

    pub(crate) fn db_table_open_json_selected(
        &mut self,
        key: &str,
        col: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(row) = self
            .db_tab_with_db(key, cx)
            .and_then(|(_, tab)| tab.selected)
        {
            self.db_table_open_json(key, row, col, window, cx);
        }
    }

    /// Open the JSON dialog on a cell: read-only when the tab is, the column is computed or the
    /// row is marked for deletion. Save puts the parsed value in the same pending overlay as any
    /// other edit (the SQL preview follows), Set NULL the column's NULL.
    fn db_table_open_json(
        &mut self,
        key: &str,
        row: usize,
        col: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let read_only = self.db_tab_read_only(key, cx);
        let Some(grid) = self.db_grid(key, cx) else {
            return;
        };
        let Some((_, tab)) = self.db_tab_with_db(key, cx) else {
            return;
        };
        if TableWidgets::of(tab).is_none_or(|widgets| widgets.json.is_some()) {
            return;
        }
        let Some(meta) = tab.columns.get(col).cloned() else {
            return;
        };
        let title = format!("{}.{}", tab.table.name, meta.name);
        let (value, deleted) = {
            let grid = grid.read(cx);
            (
                grid.cell(row, col, cx).input_text(),
                grid.row_mark(row, cx) == crate::state::db::pending::RowMark::Deleted,
            )
        };
        let spec = JsonSpec {
            title,
            read_only: read_only || meta.computed || deleted,
            meta,
            value,
        };
        let dialog = cx.new(|cx| JsonEditor::new(spec, window, cx));
        let tab_key = key.to_string();
        let subscription = cx.subscribe_in(
            &dialog,
            window,
            move |this, _, event: &JsonEvent, window, cx| {
                this.on_db_json_event(&tab_key, row, col, event.clone(), window, cx)
            },
        );
        if let Some(widgets) = self.db_tab_mut(key, cx).and_then(TableWidgets::of_mut) {
            widgets.json = Some(JsonOpen {
                dialog,
                _subscription: subscription,
            });
        }
        cx.notify();
    }

    fn on_db_json_event(
        &mut self,
        key: &str,
        row: usize,
        col: usize,
        event: JsonEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(grid) = self.db_grid(key, cx) else {
            return;
        };
        if let JsonEvent::Saved(value) = event {
            grid.update(cx, |grid, cx| grid.edit_cell(row, col, value, cx));
            self.sync_db_table_pending(key, cx);
            if self
                .db_tab_with_db(key, cx)
                .is_some_and(|(_, tab)| tab.selected == Some(row))
            {
                self.populate_db_table_form(key, window, cx);
            }
        }
        if let Some(widgets) = self.db_tab_mut(key, cx).and_then(TableWidgets::of_mut) {
            widgets.json = None;
        }
        grid.update(cx, |grid, cx| grid.focus_table(window, cx));
        cx.notify();
    }

    // ── Widgets ─────────────────────────────────────────────────────

    /// Build the widgets table tabs queued (the grid, the WHERE and ORDER BY fields), restore the
    /// tabs a saved layout names, and ask for the pages that are due.
    pub(super) fn build_db_table_widgets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        self.restore_db_tables(project);
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        let unbuilt: Vec<String> = open
            .db
            .tables
            .iter()
            .filter(|tab| tab.widgets.is_none())
            .map(|tab| tab.key.clone())
            .collect();
        for key in unbuilt {
            let widgets = self.make_db_table_widgets(&key, window, cx);
            if let Some(tab) = self.db_tab_mut(&key, cx) {
                tab.widgets = Some(Box::new(widgets));
            }
        }

        let Some(db) = self.projects.get(&project).map(|open| &open.db) else {
            return;
        };
        // The first page, once the connection list is known — a restored tab has no connection to
        // read before that. A tab that was waiting on a password asks again once it connected.
        let due: Vec<String> = db
            .tables
            .iter()
            .filter(|tab| tab.widgets.is_some() && db.loaded && db.connection(tab.conn).is_some())
            .filter(|tab| {
                !tab.requested
                    || (tab.retry
                        && !tab.loading
                        && matches!(db.state_of(tab.conn), DbConnState::Connected { .. }))
            })
            .map(|tab| tab.key.clone())
            .collect();
        let stale_forms: Vec<String> = db
            .tables
            .iter()
            .filter(|tab| tab.refill && tab.widgets.is_some())
            .map(|tab| tab.key.clone())
            .collect();
        for key in due {
            self.db_table_reload(&key, true, cx);
        }
        for key in stale_forms {
            self.refill_db_table_form(&key, window, cx);
        }
    }

    /// A saved layout names table tabs this process has no state for: give each its state once the
    /// connection it reads is known, which re-queries its first page. A tab whose connection is
    /// gone stays hidden.
    fn restore_db_tables(&mut self, project: ProjectId) {
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        if !open.db.loaded {
            return;
        }
        let restored: Vec<(DbConnId, TableRef)> = self
            .panels
            .keys()
            .filter_map(|kind| match kind {
                PanelKind::DbTable(key) if open.db.table_tab(key).is_none() => {
                    db_table_from_key(key)
                }
                _ => None,
            })
            .filter(|(conn, _)| open.db.connection(*conn).is_some())
            .collect();
        if let Some(open) = self.projects.get_mut(&project) {
            for (conn, table) in restored {
                open.db.tables.push(DbTableTab::new(conn, table));
            }
        }
    }

    fn make_db_table_widgets(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TableWidgets {
        let grid = cx.new(|cx| ResultGrid::new(window, cx));
        let where_input = cx
            .new(|cx| InputState::new(window, cx).placeholder("e.g.  status = 'open' AND total > 10"));
        let order_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("e.g.  created_at DESC, id"));
        let mut subscriptions = Vec::new();
        let tab_key = key.to_string();
        subscriptions.push(cx.subscribe_in(
            &grid,
            window,
            move |this, _, event: &GridEvent, window, cx| {
                this.on_db_grid_event(&tab_key, *event, window, cx)
            },
        ));
        for input in [&where_input, &order_input] {
            let tab_key = key.to_string();
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                move |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => this.db_table_fragments_changed(&tab_key, cx),
                    InputEvent::PressEnter { .. } => this.db_table_apply_fragments(&tab_key, cx),
                    _ => {}
                },
            ));
        }
        TableWidgets {
            grid,
            where_input,
            order_input,
            fields: Vec::new(),
            json: None,
            _subscriptions: subscriptions,
        }
    }
}
