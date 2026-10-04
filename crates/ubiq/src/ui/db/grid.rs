//! The table tab's grid: `gpui-component`'s data table over a page of rows, with inline editing.
//!
//! Rows are held as `Vec<Vec<Value>>` next to their `ColumnMeta`s on [`GridDelegate`], exactly as
//! loaded; a pending edit lives in [`Pending`] (a replaced cell, a [`RowMark`] per touched row)
//! and every cell is read through [`GridDelegate::cell`], every row's look through
//! [`GridDelegate::row_state`]. The grid tells its owner about selection with [`GridEvent`].
//!
//! **Inline editing.** The table is cell-selectable. On an `editable` grid, double-click or
//! Enter / F2 on the selected cell opens a [`CellInput`] *inside that cell's `render_td`* (the
//! delegate knows which cell is `editing`; no overlay positioned over the virtualised table). The
//! editor is the same widget as the row form's — a boolean is a dropdown (opened at once;
//! choosing commits), a date a calendar picker, a date-time the picker beside a time field — and
//! it fills the cell exactly (see `cell_input`); its popups are `deferred`, so they draw above the
//! grid. Enter commits through `parse_input` into the same pending overlay the row form writes,
//! Escape cancels (a picker that is not open lets the Escape reach the table, whose
//! `ClearSelection` is read as the cancel), Tab commits and selects the next cell, an invalid
//! value keeps the editor open with a red outline and the message in a strip under the table.
//! Clicking another cell commits a valid value and drops an invalid one. The owner turns inline
//! editing off with [`ResultGrid::set_editable`] (a read-only tab, or the form edit mode).

use std::collections::HashMap;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, Context, Div, Entity, EventEmitter, Focusable as _,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Pixels, Render,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    div, px,
};
use gpui_component::table::{Column, DataTable, TableDelegate, TableEvent, TableState};
use gpui_component::tooltip::Tooltip;
use gpui_component::{Sizable as _, Size};
use ubiq_db::value::parse_input;
use ubiq_proto::db::{ColumnMeta, DataType, ResultSet, RowEdit, Value};

use super::cell_input::{CellInput, CellInputEvent};
use super::json::json;
use crate::state::db::pending::Pending;
pub use crate::state::db::pending::RowMark;
use crate::theme::{self, Family, Role};

/// Longest cell text drawn; the value itself is kept whole.
const MAX_CELL_CHARS: usize = 300;
/// Rows sampled to size a column.
const WIDTH_SAMPLE_ROWS: usize = 50;
/// A character's width as a fraction of the content font size, for sizing a column.
const CHAR_WIDTH_EM: f32 = 0.58;
const MIN_COL_WIDTH: f32 = 60.0;
const MAX_COL_WIDTH: f32 = 420.0;
/// The grid's row size: the densest the table offers. `cell_input`'s inline editor pads for it.
pub const GRID_SIZE: Size = Size::XSmall;
/// The type role of a cell, a header and the inline editor over a cell.
pub const CELL_ROLE: Role = Role::Meta;

/// What the grid tells its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridEvent {
    /// The selected row changed (or another cell of it was selected); `None` = nothing selected
    /// any more.
    Selected(Option<usize>),
    /// An inline edit was committed into the pending overlay of this cell.
    Edited { row: usize, col: usize },
    /// This cell is edited in the JSON dialog (a `json` column, a text one holding JSON, or the
    /// `{ }` button), editable grid or not: the owner opens it, read-only when the tab is.
    OpenJson { row: usize, col: usize },
}

/// The cell being edited inline, and its input.
#[derive(Clone)]
struct Editing {
    row: usize,
    col: usize,
    input: Entity<CellInput>,
}

/// The table's data source: the page's columns and rows plus the pending edits.
#[derive(Default)]
pub struct GridDelegate {
    columns: Vec<ColumnMeta>,
    rows: Vec<Vec<Value>>,
    widths: Vec<Pixels>,
    pending: Pending,
    editing: Option<Editing>,
    /// Why a row's change was refused by the server, by row.
    errors: HashMap<usize, String>,
}

impl GridDelegate {
    fn load(&mut self, rs: ResultSet) {
        self.widths = rs
            .columns
            .iter()
            .enumerate()
            .map(|(ix, meta)| column_width(meta, rs.rows.iter().take(WIDTH_SAMPLE_ROWS), ix))
            .collect();
        self.columns = rs.columns;
        self.rows = rs.rows;
        self.pending.reset(self.rows.len());
        self.editing = None;
        self.errors.clear();
    }

    /// The cell as loaded, ignoring any pending value.
    pub fn original(&self, row: usize, col: usize) -> &Value {
        static NULL: Value = Value::Null;
        self.rows.get(row).and_then(|r| r.get(col)).unwrap_or(&NULL)
    }

    /// The cell as shown: the pending value if one is overlaid, else the stored one.
    pub fn cell(&self, row: usize, col: usize) -> &Value {
        self.pending
            .overlay(row, col)
            .unwrap_or_else(|| self.original(row, col))
    }

    /// How row `ix` is drawn.
    pub fn row_state(&self, ix: usize) -> RowMark {
        self.pending.mark(ix)
    }
}

fn column_width<'a>(
    meta: &ColumnMeta,
    sample: impl Iterator<Item = &'a Vec<Value>>,
    ix: usize,
) -> Pixels {
    let mut chars = meta.name.chars().count();
    for row in sample {
        if let Some(v) = row.get(ix) {
            chars = chars.max(shown(meta, v).chars().take(MAX_CELL_CHARS).count());
        }
    }
    let em = f32::from(theme::font(Family::Content, CELL_ROLE));
    px((chars as f32 * em * CHAR_WIDTH_EM + theme::scaled(24.0))
        .clamp(theme::scaled(MIN_COL_WIDTH), theme::scaled(MAX_COL_WIDTH)))
}

/// The cell's text before it is cut to one line: a `json` cell is compacted (no blanks outside
/// strings), the stored value is untouched.
fn shown(meta: &ColumnMeta, value: &Value) -> String {
    if matches!(meta.data_type, DataType::Json) && !matches!(value, Value::Null) {
        json::preview(&value.to_string(), MAX_CELL_CHARS + 1)
    } else {
        value.to_string()
    }
}

/// One line of at most [`MAX_CELL_CHARS`] characters: line breaks become `⏎`.
fn cell_text(meta: &ColumnMeta, value: &Value) -> String {
    let text = shown(meta, value);
    let mut out: String = text
        .chars()
        .take(MAX_CELL_CHARS)
        .map(|c| {
            if matches!(c, '\n' | '\r' | '\t') {
                '⏎'
            } else {
                c
            }
        })
        .collect();
    if text.chars().nth(MAX_CELL_CHARS).is_some() {
        out.push('…');
    }
    out
}

impl TableDelegate for GridDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let meta = &self.columns[col_ix];
        let column = Column::new(col_ix.to_string(), meta.name.clone())
            .width(self.widths[col_ix])
            .min_width(px(theme::scaled(40.0)));
        if meta.data_type.is_numeric() {
            column.text_right()
        } else {
            column
        }
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let meta = &self.columns[col_ix];
        let tip = SharedString::from(if meta.native_type.is_empty() {
            meta.name.clone()
        } else {
            format!("{} · {}", meta.name, meta.native_type)
        });
        div()
            .id(("db-grid-th", col_ix))
            .size_full()
            .flex()
            .items_center()
            .text_size(theme::font(Family::Content, CELL_ROLE))
            .when(meta.data_type.is_numeric(), |this| this.justify_end())
            .child(SharedString::from(meta.name.clone()))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let row = div().id(("db-grid-row", row_ix));
        let row = match self.row_state(row_ix) {
            RowMark::Clean => row,
            RowMark::Edited => row.bg(theme::db_row_edited()),
            RowMark::Inserted => row.bg(theme::db_row_inserted()),
            RowMark::Deleted => row
                .bg(theme::db_row_deleted())
                .text_color(theme::text_faint())
                .line_through(),
        };
        // A row the server refused wears the status edge.
        if self.errors.contains_key(&row_ix) {
            row.border_l(px(theme::accent_edge()))
                .border_color(theme::danger())
        } else {
            row
        }
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        if let Some(editing) = self
            .editing
            .as_ref()
            .filter(|e| (e.row, e.col) == (row_ix, col_ix))
        {
            // The editor is positioned over the whole cell (see `cell_input`): this box only
            // gives it a parent to be absolute in, and takes no room itself.
            return div()
                .relative()
                .size_full()
                .child(editing.input.clone())
                .into_any_element();
        }
        let value = self.cell(row_ix, col_ix);
        let changed = self.pending.overlay(row_ix, col_ix).is_some();
        let null = matches!(value, Value::Null);
        // `''` is not NULL: show it as a faint pair of quotes rather than as nothing.
        let empty = matches!(value, Value::Text(s) if s.is_empty());
        let numeric = self.columns[col_ix].data_type.is_numeric();
        let text = if null {
            SharedString::from("NULL")
        } else if empty {
            SharedString::from("''")
        } else {
            SharedString::from(cell_text(&self.columns[col_ix], value))
        };
        div()
            .size_full()
            .flex()
            .items_center()
            .font_family(theme::MONO_FONT)
            .text_size(theme::font(Family::Content, CELL_ROLE))
            .when(numeric, |this| this.justify_end())
            .when(null, |this| this.italic().text_color(theme::text_faint()))
            .when(empty, |this| this.text_color(theme::text_faint()))
            .when(!null && !empty, |this| this.text_color(theme::text()))
            .when(changed, |this| {
                this.text_color(theme::accent())
                    .font_weight(gpui::FontWeight::BOLD)
            })
            .child(
                div()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(text),
            )
            .into_any_element()
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(theme::font(Family::Content, Role::Dense))
            .text_color(theme::text_faint())
            .child("No rows")
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        self.cell(row_ix, col_ix).to_string()
    }
}

/// The grid entity: a data table over one page.
pub struct ResultGrid {
    state: Entity<TableState<GridDelegate>>,
    /// Whether cells can be edited inline (a table of a read-write connection, not locked).
    editable: bool,
    /// The selected row, from whichever selection the table last made.
    selected: Option<usize>,
    /// Listens to the inline editor while there is one.
    edit_subscription: Option<Subscription>,
    _subscription: Subscription,
}

impl EventEmitter<GridEvent> for ResultGrid {}

impl ResultGrid {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| {
            TableState::new(GridDelegate::default(), window, cx)
                .sortable(false)
                .cell_selectable(true)
        });
        let subscription = cx.subscribe_in(
            &state,
            window,
            |this, _, event: &TableEvent, window, cx| match event {
                TableEvent::SelectRow(ix) => {
                    this.settle_edit(None, window, cx);
                    this.select(Some(*ix), cx);
                }
                TableEvent::SelectCell(row, col) => {
                    this.settle_edit(Some((*row, *col)), window, cx);
                    this.select(Some(*row), cx);
                }
                TableEvent::DoubleClickedCell(row, col) => this.start_edit(*row, *col, window, cx),
                TableEvent::ClearSelection => {
                    // Escape that a dropdown or a date picker did not use (closed) reaches the
                    // table's own Cancel, which clears the selection: here that means "cancel
                    // the edit", and the cell stays selected.
                    if let Some(Editing { row, col, .. }) = this.editing(cx) {
                        this.close_edit(true, window, cx);
                        this.state
                            .update(cx, |table, cx| table.set_selected_cell(row, col, cx));
                    } else {
                        this.select(None, cx);
                    }
                }
                _ => {}
            },
        );
        Self {
            state,
            editable: false,
            selected: None,
            edit_subscription: None,
            _subscription: subscription,
        }
    }

    /// Allow (or refuse) inline editing; a grid starts read-only. Refusing settles an editor that
    /// is open (a valid value is committed, an invalid one dropped).
    pub fn set_editable(&mut self, editable: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.editable = editable;
        if !editable {
            self.settle_edit(None, window, cx);
        }
    }

    fn select(&mut self, row: Option<usize>, cx: &mut Context<Self>) {
        self.selected = row;
        cx.emit(GridEvent::Selected(row));
    }

    // ── Inline editing ──────────────────────────────────────────────

    fn editing(&self, cx: &App) -> Option<Editing> {
        self.state.read(cx).delegate().editing.clone()
    }

    /// Open the inline editor on a cell, if the grid is editable and the cell can be.
    pub fn start_edit(
        &mut self,
        row: usize,
        col: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A JSON value goes to the dialog, which also views a cell of a read-only grid.
        let routed = {
            let d = self.state.read(cx).delegate();
            row < d.rows.len()
                && d.columns.get(col).is_some_and(|meta| {
                    json::route(&meta.data_type, d.cell(row, col).input_text().as_deref())
                })
        };
        if routed {
            self.settle_edit(None, window, cx);
            cx.emit(GridEvent::OpenJson { row, col });
            return;
        }
        if !self.editable {
            return;
        }
        if let Some(editing) = self.editing(cx) {
            if (editing.row, editing.col) == (row, col) {
                return;
            }
            self.settle_edit(None, window, cx);
        }
        let d = self.state.read(cx).delegate();
        let Some(meta) = d.columns.get(col).cloned() else {
            return;
        };
        if row >= d.rows.len() || meta.computed || d.row_state(row) == RowMark::Deleted {
            return;
        }
        let value = d.cell(row, col).input_text();
        let input = cx.new(|cx| CellInput::new(&meta, true, window, cx));
        input.update(cx, |input, cx| {
            input.load(value.as_deref(), window, cx);
            input.focus(window, cx);
            // A boolean opens its dropdown (a date its calendar) straight away.
            input.open_picker(window, cx);
        });
        self.edit_subscription = Some(cx.subscribe_in(
            &input,
            window,
            |this, _, event: &CellInputEvent, window, cx| match *event {
                CellInputEvent::Commit => {
                    this.commit_edit(window, cx);
                }
                CellInputEvent::Cancel => this.close_edit(true, window, cx),
                CellInputEvent::Tab { back } => {
                    if this.commit_edit(window, cx) {
                        this.step(back, cx);
                    }
                }
                CellInputEvent::Changed => cx.notify(),
                CellInputEvent::OpenJson => {
                    // The text typed so far is committed, then the dialog edits the cell.
                    if let Some(Editing { row, col, .. }) = this.editing(cx)
                        && this.commit_edit(window, cx)
                    {
                        cx.emit(GridEvent::OpenJson { row, col });
                    }
                }
            },
        ));
        self.state.update(cx, |table, cx| {
            table.delegate_mut().editing = Some(Editing { row, col, input });
            cx.notify();
        });
        cx.notify();
    }

    /// Parse the editor's value into the pending overlay and close it. False = invalid: the
    /// editor stays open with the message.
    fn commit_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(Editing { row, col, input }) = self.editing(cx) else {
            return true;
        };
        let Some(meta) = self.state.read(cx).delegate().columns.get(col).cloned() else {
            return true;
        };
        let value = input.read(cx).value(cx);
        match parse_input(value.as_deref(), &meta) {
            Ok(value) => {
                self.edit_cell(row, col, value, cx);
                self.close_edit(true, window, cx);
                cx.emit(GridEvent::Edited { row, col });
                true
            }
            Err(error) => {
                input.update(cx, |input, cx| input.set_error(Some(error.message), cx));
                cx.notify();
                false
            }
        }
    }

    /// Put the keyboard back on the table (after a dialog closes).
    pub fn focus_table(&self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.state.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    /// Drop the editor; `refocus` puts the keyboard back on the table.
    fn close_edit(&mut self, refocus: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_subscription = None;
        self.state.update(cx, |table, cx| {
            table.delegate_mut().editing = None;
            cx.notify();
        });
        if refocus {
            let handle = self.state.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }
        cx.notify();
    }

    /// The selection moved to `to` (`None` = a row or nothing): a valid edit in progress is
    /// committed, an invalid one dropped. Moving within the edited cell changes nothing.
    fn settle_edit(
        &mut self,
        to: Option<(usize, usize)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editing) = self.editing(cx) else {
            return;
        };
        if to == Some((editing.row, editing.col)) {
            return;
        }
        if !self.commit_edit(window, cx) {
            self.close_edit(false, window, cx);
        }
    }

    /// Select the next (or previous) cell, wrapping to the next row.
    fn step(&mut self, back: bool, cx: &mut Context<Self>) {
        self.state.update(cx, |table, cx| {
            let Some((row, col)) = table.selected_cell() else {
                return;
            };
            let (cols, rows) = {
                let d = table.delegate();
                (d.columns.len(), d.rows.len())
            };
            let next = match (back, col + 1 < cols, col > 0) {
                (false, true, _) => Some((row, col + 1)),
                (false, false, _) if row + 1 < rows => Some((row + 1, 0)),
                (true, _, true) => Some((row, col - 1)),
                (true, _, false) if row > 0 => Some((row - 1, cols.saturating_sub(1))),
                _ => None,
            };
            if let Some((row, col)) = next {
                table.set_selected_cell(row, col, cx);
            }
        });
    }

    /// Enter / F2 on the selected cell opens the editor.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = event.keystroke.modifiers;
        if self.editing(cx).is_some() || m.control || m.alt || m.platform || m.shift {
            return;
        }
        if !matches!(event.keystroke.key.as_str(), "enter" | "f2") {
            return;
        }
        if self.edit_selected(window, cx) {
            cx.stop_propagation();
        }
    }

    /// Edit the selected cell, if one is selected — the `DbEditCell` action's answer (F2) as well
    /// as Enter's. False = nothing was selected.
    pub fn edit_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((row, col)) = self.state.read(cx).selected_cell() else {
            return false;
        };
        self.start_edit(row, col, window, cx);
        true
    }

    // ── Editing ─────────────────────────────────────────────────────

    /// The cell as loaded.
    pub fn original(&self, row: usize, col: usize, cx: &App) -> Value {
        self.state.read(cx).delegate().original(row, col).clone()
    }

    /// The cell as shown (pending value, else loaded).
    pub fn cell(&self, row: usize, col: usize, cx: &App) -> Value {
        self.state.read(cx).delegate().cell(row, col).clone()
    }

    pub fn row_mark(&self, row: usize, cx: &App) -> RowMark {
        self.state.read(cx).delegate().row_state(row)
    }

    /// Set a cell's pending value; the loaded value there drops the pending one.
    pub fn edit_cell(&mut self, row: usize, col: usize, value: Value, cx: &mut Context<Self>) {
        self.state.update(cx, |table, cx| {
            let d = table.delegate_mut();
            let original = d.original(row, col).clone();
            d.pending.set(row, col, value, &original);
            d.errors.remove(&row);
            cx.notify();
        });
        cx.notify();
    }

    /// Append a new (all NULL) row, select it, and return its index.
    pub fn add_row(&mut self, cx: &mut Context<Self>) -> usize {
        let ix = self.state.update(cx, |table, cx| {
            let d = table.delegate_mut();
            let ix = d.rows.len();
            d.rows.push(vec![Value::Null; d.columns.len()]);
            d.pending.add_row(ix);
            table.refresh(cx);
            table.set_selected_cell(ix, 0, cx);
            ix
        });
        cx.notify();
        ix
    }

    /// Mark a row deleted, or take the mark back.
    pub fn toggle_delete(&mut self, row: usize, cx: &mut Context<Self>) {
        self.state.update(cx, |table, cx| {
            let d = table.delegate_mut();
            d.pending.toggle_delete(row);
            if d.editing.as_ref().is_some_and(|e| e.row == row) {
                d.editing = None;
            }
            cx.notify();
        });
        self.edit_subscription = None;
        cx.notify();
    }

    /// Drop every pending change, appended rows included, and any inline editor.
    pub fn discard(&mut self, cx: &mut Context<Self>) {
        self.edit_subscription = None;
        self.state.update(cx, |table, cx| {
            let d = table.delegate_mut();
            d.editing = None;
            d.rows.truncate(d.pending.loaded());
            d.pending.reset(d.rows.len());
            d.errors.clear();
            table.refresh(cx);
            cx.notify();
        });
        cx.notify();
    }

    /// The pending changes as row edits, each with the row it came from.
    pub fn edits_by_row(&self, cx: &App) -> Vec<(usize, RowEdit)> {
        let d = self.state.read(cx).delegate();
        d.pending.edits_by_row(&d.columns, &d.rows)
    }

    /// The pending changes as row edits.
    pub fn edits(&self, cx: &App) -> Vec<RowEdit> {
        self.edits_by_row(cx)
            .into_iter()
            .map(|(_, edit)| edit)
            .collect()
    }

    /// Rows with anything pending.
    pub fn pending_rows(&self, cx: &App) -> usize {
        self.state.read(cx).delegate().pending.count()
    }

    /// The row the server refused, and why — `None` clears the mark.
    pub fn set_row_error(&mut self, row: usize, error: Option<String>, cx: &mut Context<Self>) {
        self.state.update(cx, |table, cx| {
            let d = table.delegate_mut();
            match error {
                Some(error) => d.errors.insert(row, error),
                None => d.errors.remove(&row),
            };
            cx.notify();
        });
        cx.notify();
    }

    /// Why the server refused this row's change, if it did.
    pub fn row_error(&self, row: usize, cx: &App) -> Option<String> {
        self.state.read(cx).delegate().errors.get(&row).cloned()
    }

    /// Replace the shown page and drop the selection.
    pub fn set_result(&mut self, rs: ResultSet, cx: &mut Context<Self>) {
        self.edit_subscription = None;
        self.selected = None;
        self.state.update(cx, |table, cx| {
            table.delegate_mut().load(rs);
            table.clear_selection(cx);
            table.refresh(cx);
            cx.notify();
        });
        cx.notify();
    }
}

impl Render for ResultGrid {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The inline editor's complaint (invalid value) or hint (NOT NULL), under the table.
        let message = self.editing(cx).and_then(|e| {
            let input = e.input.read(cx);
            input
                .error()
                .map(|m| (theme::danger(), m.clone()))
                .or_else(|| input.hint().map(|m| (theme::warning(), m.clone())))
        });
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key_down(event, window, cx)
            }))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .child(
                        DataTable::new(&self.state)
                            .with_size(GRID_SIZE)
                            .stripe(true)
                            .bordered(false),
                    ),
            )
            .when_some(message, |this, (color, text)| {
                this.child(
                    div()
                        .flex_none()
                        .px_2()
                        .py_1()
                        .border_t(px(theme::hairline()))
                        .border_color(theme::border())
                        .text_size(theme::font(Family::Content, Role::Dense))
                        .text_color(color)
                        .child(text),
                )
            })
    }
}
