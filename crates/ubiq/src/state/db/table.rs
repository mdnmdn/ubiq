//! One open table tab: what it is looking at and the page it last drew.
//!
//! The tab is a session (`session`), so every request it sends carries that id and a reply for a
//! tab since closed is discarded by it; inside the tab, `query` names the one run whose reply is
//! still wanted, so a page that was superseded by a refresh is dropped for the same reason.
//!
//! **What lives here and what does not.** The page's position, the applied WHERE and ORDER BY, the
//! counts, the lock, the edit mode and the note are plain data, and so are their rules — read-only
//! is derived ([`DbTableTab::read_only`]), a reply is accepted or refused ([`DbTableTab::accept_page`]).
//! The grid, the cell editors and the JSON dialog are widgets that need a `Window`; they are the
//! interface's and ride in [`DbTableTab::widgets`] as an opaque box, so this module names no
//! widget. The pending edits themselves are the grid's ([`super::pending`]); [`DbTableTab::pending_rows`]
//! mirrors their count for the tab strip and the toolbar, which have no grid to ask.

use std::any::Any;

use ubiq_proto::db::{ColumnMeta, DbFailure, DbFailureKind, DbPage, ResultSet, TableRef};
use ubiq_proto::ids::{DbConnId, DbQueryId, DbSessionId};

/// The page sizes the toolbar steps through; the first request uses [`DEFAULT_PAGE_SIZE`].
pub const PAGE_SIZES: [u32; 5] = [50, 100, 200, 500, 1000];
/// Rows per page when a tab opens.
pub const DEFAULT_PAGE_SIZE: u32 = 200;

/// Where an editable tab edits: **Inline** edits cells in the grid and hides the side form;
/// **Form** makes the grid selection-only and shows the side form for the selected row. Both write
/// the same pending overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditMode {
    #[default]
    Inline,
    Form,
}

/// The message strip under the bars.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableNote {
    pub error: bool,
    pub text: String,
}

impl TableNote {
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            error: true,
            text: text.into(),
        }
    }

    pub fn info(text: impl Into<String>) -> Self {
        Self {
            error: false,
            text: text.into(),
        }
    }
}

pub struct DbTableTab {
    /// The panel's key — [`super::db_table_key`].
    pub key: String,
    pub session: DbSessionId,
    pub conn: DbConnId,
    pub table: TableRef,
    /// The run whose reply this tab is waiting for; a reply naming any other is stale. A page and
    /// an apply never overlap (`loading`/`applying` gate each other), so one slot serves both.
    pub query: Option<DbQueryId>,
    /// Whether the first page has been asked for. A restored tab asks once the connection list
    /// has arrived.
    pub requested: bool,
    /// Tables are editable; a view or synonym is not, whatever the connection says.
    pub editable: bool,
    /// The per-tab lock. Session state, not persisted.
    pub locked: bool,
    /// The tree's row estimate, shown until the exact count arrives.
    pub approx_rows: Option<u64>,
    /// The catalog's columns of the loaded page.
    pub columns: Vec<ColumnMeta>,
    /// What the loaded page was fetched with.
    pub filter: String,
    pub order_by: String,
    /// The validation error of the WHERE / ORDER BY boxes, shown next to them.
    pub fragment_error: Option<String>,
    /// Zero-based page.
    pub page: u64,
    pub page_size: u32,
    pub has_more: bool,
    /// Rows the loaded page holds (without appended ones).
    pub page_rows: usize,
    /// The exact count, once it arrives.
    pub total: Option<u64>,
    pub loading: bool,
    pub applying: bool,
    /// Statements in the batch being applied, for the confirmation line.
    pub applying_count: usize,
    pub note: Option<TableNote>,
    /// Said after the next load ("Applied 3 statements").
    pub carry: Option<String>,
    pub selected: Option<usize>,
    /// Rows with anything pending — the grid's count, mirrored.
    pub pending_rows: usize,
    pub edit_mode: EditMode,
    /// The row inspector of a read-only tab is shown.
    pub show_form: bool,
    pub show_sql: bool,
    /// Per column of the row form: why its text was refused.
    pub field_errors: Vec<Option<String>>,
    /// The page changed under the row form: its fields are rebuilt from `columns` and refilled.
    pub refill: bool,
    /// The connection asked for a password; the page is asked again once it has connected.
    pub retry: bool,
    /// The widgets this tab owns, built in `render` where there is a `Window`. Opaque here.
    pub widgets: Option<Box<dyn Any>>,
}

impl DbTableTab {
    pub fn new(conn: DbConnId, table: TableRef) -> Self {
        Self {
            key: super::db_table_key(conn, &table),
            session: DbSessionId::generate(),
            conn,
            table,
            query: None,
            requested: false,
            editable: true,
            locked: false,
            approx_rows: None,
            columns: Vec::new(),
            filter: String::new(),
            order_by: String::new(),
            fragment_error: None,
            page: 0,
            page_size: DEFAULT_PAGE_SIZE,
            has_more: false,
            page_rows: 0,
            total: None,
            loading: false,
            applying: false,
            applying_count: 0,
            note: None,
            carry: None,
            selected: None,
            pending_rows: 0,
            edit_mode: EditMode::Inline,
            show_form: true,
            show_sql: false,
            field_errors: Vec::new(),
            refill: false,
            retry: false,
            widgets: None,
        }
    }

    // ── Read-only ───────────────────────────────────────────────────

    /// The lock cannot be turned off: the connection is read-only, or this is a view or synonym.
    pub fn forced(&self, connection_read_only: bool) -> bool {
        connection_read_only || !self.editable
    }

    /// Whether the tab is read-only now: forced, or locked by the toggle.
    pub fn read_only(&self, connection_read_only: bool) -> bool {
        self.forced(connection_read_only) || self.locked
    }

    /// Rows can be edited.
    pub fn can_edit(&self, connection_read_only: bool) -> bool {
        !self.read_only(connection_read_only)
    }

    /// The side form is shown: in Form mode of an editable tab always, in a read-only tab when the
    /// inspector is toggled on — and only with a row selected.
    pub fn form_visible(&self, connection_read_only: bool) -> bool {
        let wanted = if self.can_edit(connection_read_only) {
            self.edit_mode == EditMode::Form
        } else {
            self.show_form
        };
        wanted && self.selected.is_some()
    }

    /// Whether cells edit in the grid: the tab can edit and the mode is Inline.
    pub fn grid_editable(&self, connection_read_only: bool) -> bool {
        self.can_edit(connection_read_only) && self.edit_mode == EditMode::Inline
    }

    // ── Paging ──────────────────────────────────────────────────────

    /// The row the current page starts at.
    pub fn offset(&self) -> u64 {
        self.page * u64::from(self.page_size)
    }

    /// The next size in [`PAGE_SIZES`], wrapping.
    pub fn next_page_size(&self) -> u32 {
        let at = PAGE_SIZES.iter().position(|size| *size == self.page_size);
        PAGE_SIZES[at.map_or(0, |at| (at + 1) % PAGE_SIZES.len())]
    }

    pub fn can_prev(&self) -> bool {
        self.page > 0 && !self.loading
    }

    pub fn can_next(&self) -> bool {
        self.has_more && !self.loading
    }

    /// Navigation drops the page's pending edits, so it refuses while there are any.
    pub fn leave_blocked(&self) -> bool {
        self.pending_rows > 0
    }

    /// "1–200", or "no rows".
    pub fn range_label(&self) -> String {
        if self.page_rows == 0 {
            return "no rows".to_string();
        }
        let from = self.offset();
        format!("{}–{}", from + 1, from + self.page_rows as u64)
    }

    /// The range and the count: the exact one once it arrives; until then the tree's estimate
    /// (only meaningful without a filter, since it counts the whole table).
    pub fn count_label(&self) -> String {
        let range = self.range_label();
        let estimate = self.approx_rows.filter(|_| self.filter.is_empty());
        match (self.total, estimate) {
            (Some(total), _) => format!("{range} of {total}"),
            (None, Some(rows)) if self.loading => {
                format!("loading… · {} rows (estimate)", humanise_rows(rows))
            }
            (None, Some(rows)) => format!("{range} · {} rows (estimate)", humanise_rows(rows)),
            (None, None) if self.loading => "loading…".to_string(),
            (None, None) => range,
        }
    }

    // ── Loading ─────────────────────────────────────────────────────

    /// A page is being asked for: `query` is the run to wait for. With `recount` the old exact
    /// count is forgotten, since the new request asks for it again.
    pub fn begin_load(&mut self, query: DbQueryId, recount: bool) {
        self.requested = true;
        self.retry = false;
        self.query = Some(query);
        self.loading = true;
        if recount {
            self.total = None;
        }
    }

    /// Take a `DbTablePageResult`. `None` = nothing to draw: the reply was stale, or it failed
    /// (the note says why). `Some` is the rows to show, already cut to one page; the tab's page
    /// facts are updated to match.
    pub fn accept_page(
        &mut self,
        query: DbQueryId,
        elapsed_ms: u64,
        result: Result<Box<DbPage>, DbFailure>,
    ) -> Option<ResultSet> {
        if self.query != Some(query) {
            return None;
        }
        self.query = None;
        self.loading = false;
        match result {
            Err(failure) => {
                self.carry = None;
                self.retry = failure.kind == DbFailureKind::NeedsPassword;
                self.note = Some(TableNote::error(describe_failure(&failure)));
                None
            }
            Ok(page) => {
                let DbPage {
                    columns,
                    mut rows,
                    exact_count,
                } = *page;
                let size = self.page_size as usize;
                self.has_more = rows.rows.len() > size;
                rows.rows.truncate(size);
                rows.columns = if rows.columns.is_empty() {
                    columns.clone()
                } else {
                    // The catalog's columns carry key, nullability and type; the result's only
                    // name and what the driver saw.
                    rows.columns
                        .iter()
                        .map(|c| {
                            columns
                                .iter()
                                .find(|m| m.name == c.name)
                                .cloned()
                                .unwrap_or_else(|| c.clone())
                        })
                        .collect()
                };
                if exact_count.is_some() {
                    self.total = exact_count;
                }
                let text = format!("{} rows · {elapsed_ms} ms", rows.rows.len());
                let text = match self.carry.take() {
                    Some(lead) => format!("{lead} · {text}"),
                    None => text,
                };
                self.note = Some(TableNote::info(text));
                self.columns = rows.columns.clone();
                self.page_rows = rows.rows.len();
                self.selected = None;
                self.pending_rows = 0;
                self.refill = true;
                Some(rows)
            }
        }
    }

    /// The batch was applied and committed.
    pub fn applied(&mut self, affected: u64) {
        self.applying = false;
        self.query = None;
        self.carry = Some(format!(
            "Applied {} statement{} ({affected} rows affected)",
            self.applying_count,
            if self.applying_count == 1 { "" } else { "s" }
        ));
    }
}

/// A failure for the note strip. A read-only refusal says so plainly and that nothing ran.
pub fn describe_failure(failure: &DbFailure) -> String {
    match failure.kind {
        DbFailureKind::ReadOnly => format!("Read-only: nothing was written. {}", failure.message),
        DbFailureKind::NeedsPassword => "The connection needs a password.".to_string(),
        _ => failure.message.clone(),
    }
}

/// An estimated row count as the explorer and the toolbar write it: `842`, `12.3k`, `4.1M`.
pub fn humanise_rows(rows: u64) -> String {
    match rows {
        0..=999 => rows.to_string(),
        1_000..=999_999 => format!("{:.1}k", rows as f64 / 1_000.0),
        _ => format!("{:.1}M", rows as f64 / 1_000_000.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::db::{DbKind, Value};

    fn tab() -> DbTableTab {
        DbTableTab::new(DbConnId::generate(), TableRef::new(None, None, "orders"))
    }

    fn page(rows: usize, exact: Option<u64>) -> Box<DbPage> {
        let columns = vec![ColumnMeta::new(DbKind::Sqlite, "id", "INTEGER")];
        Box::new(DbPage {
            columns: columns.clone(),
            rows: ResultSet {
                columns: Vec::new(),
                rows: (0..rows).map(|n| vec![Value::Int(n as i64)]).collect(),
                truncated: false,
            },
            exact_count: exact,
        })
    }

    #[test]
    fn a_connection_or_a_view_forces_the_lock_and_the_toggle_adds_to_it() {
        let mut tab = tab();
        assert!(tab.can_edit(false));
        tab.locked = true;
        assert!(tab.read_only(false) && !tab.forced(false));
        tab.locked = false;
        assert!(tab.read_only(true) && tab.forced(true));
        tab.editable = false;
        assert!(tab.read_only(false) && tab.forced(false));
    }

    #[test]
    fn the_form_is_for_form_mode_or_the_inspector_and_needs_a_row() {
        let mut tab = tab();
        tab.selected = Some(0);
        assert!(!tab.form_visible(false), "inline mode hides the form");
        tab.edit_mode = EditMode::Form;
        assert!(tab.form_visible(false));
        tab.selected = None;
        assert!(!tab.form_visible(false));
        tab.selected = Some(1);
        tab.edit_mode = EditMode::Inline;
        assert!(
            tab.form_visible(true),
            "a read-only tab shows the inspector"
        );
        tab.show_form = false;
        assert!(!tab.form_visible(true));
    }

    #[test]
    fn a_stale_reply_is_dropped_and_the_wanted_one_is_cut_to_a_page() {
        let mut tab = tab();
        tab.page_size = 50;
        let (old, new) = (DbQueryId::generate(), DbQueryId::generate());
        tab.begin_load(old, true);
        tab.begin_load(new, true);
        assert!(tab.accept_page(old, 3, Ok(page(10, None))).is_none());
        assert!(tab.loading, "the stale reply left the wait alone");
        // One row more than a page came back: that is "there is a next page".
        let rows = tab.accept_page(new, 7, Ok(page(51, Some(120)))).unwrap();
        assert_eq!(rows.rows.len(), 50);
        assert!(tab.has_more && !tab.loading);
        assert_eq!(tab.total, Some(120));
        assert_eq!(tab.columns.len(), 1);
        assert!(tab.refill);
        assert_eq!(tab.count_label(), "1–50 of 120");
    }

    #[test]
    fn a_failure_is_a_note_and_a_password_one_waits_for_a_connection() {
        let mut tab = tab();
        let query = DbQueryId::generate();
        tab.begin_load(query, false);
        let failure = DbFailure::new(DbFailureKind::NeedsPassword, "no password");
        assert!(tab.accept_page(query, 1, Err(failure)).is_none());
        assert!(tab.retry && tab.note.as_ref().is_some_and(|n| n.error));
        let query = DbQueryId::generate();
        tab.begin_load(query, false);
        assert!(!tab.retry, "asking again ends the wait");
        let refused = DbFailure::new(DbFailureKind::ReadOnly, "UPDATE is a write");
        tab.accept_page(query, 1, Err(refused));
        let note = tab.note.unwrap();
        assert!(
            note.text.starts_with("Read-only: nothing was written."),
            "{note:?}"
        );
    }

    #[test]
    fn the_estimate_stands_in_for_the_count_until_it_arrives_and_only_without_a_filter() {
        let mut tab = tab();
        tab.approx_rows = Some(12_345);
        tab.page_rows = 200;
        assert_eq!(tab.count_label(), "1–200 · 12.3k rows (estimate)");
        tab.filter = "id > 3".into();
        assert_eq!(tab.count_label(), "1–200");
        tab.total = Some(9);
        assert_eq!(tab.count_label(), "1–200 of 9");
    }

    #[test]
    fn paging_moves_the_offset_and_a_size_change_steps_the_ladder() {
        let mut tab = tab();
        assert_eq!(tab.offset(), 0);
        tab.page = 2;
        assert_eq!(tab.offset(), 400);
        assert_eq!(tab.next_page_size(), 500);
        tab.page_size = 1000;
        assert_eq!(tab.next_page_size(), 50);
        assert_eq!(humanise_rows(842), "842");
        assert_eq!(humanise_rows(4_100_000), "4.1M");
    }

    #[test]
    fn an_applied_batch_is_said_after_the_next_load() {
        let mut tab = tab();
        tab.applying = true;
        tab.applying_count = 3;
        tab.applied(5);
        let query = DbQueryId::generate();
        tab.begin_load(query, true);
        tab.accept_page(query, 2, Ok(page(1, None)));
        assert_eq!(
            tab.note.unwrap().text,
            "Applied 3 statements (5 rows affected) · 1 rows · 2 ms"
        );
    }
}
