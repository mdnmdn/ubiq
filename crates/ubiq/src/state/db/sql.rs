//! One open SQL tab: its connection, current database and text, the run it is waiting on, and what
//! the runs left behind.
//!
//! The tab is a session (`session`) for the reason a table tab is: it mints a [`DbSessionId`] when
//! it opens and every request carries it, so a reply for a tab the user has closed finds nothing.
//! Each run also mints a [`DbQueryId`] — held in `query` — which is both the handle Stop cancels by
//! and the check that a reply belongs to *this* run and not an earlier one.
//!
//! **What Run would do is a pure question** ([`pick`]): the selection if there is one, else the
//! statement under the cursor ([`ubiq_db::sql::statement_at`]), else — for Run all — every
//! statement. It lives here, beside the tab, so the answer is tested without a window.
//!
//! The editor and the result grids are widgets that need a `Window` to be built; the tab holds them
//! as `Option`s and `AppState::build_db_sql_widgets` fills them in `render`, `pending_kb_docs`'
//! pattern. Until then `text` is the editor's seed, and afterwards its last known contents (what a
//! draft saves).

use std::ops::Range;
use std::time::Instant;

use gpui::{Entity, SharedString, Subscription};
use gpui_component::input::{EditorState, TextDecorationCollection};
use gpui_component::table::TableState;
use ubiq_db::sql::{self, Analysis, Dialect, ReadOnlyRule, StatementClass};
use ubiq_proto::db::{DbEditor, DbFailure, DbFailureKind, DbOutcome, ResultSet};
use ubiq_proto::ids::{DbConnId, DbQueryId, DbSessionId};

use crate::ui::db::sql::plan::PlanState;
use crate::ui::db::sql::results::GridDelegate;

/// Quiet time, in milliseconds, after the last keystroke before the text is analysed again.
pub const ANALYSIS_DEBOUNCE_MS: u64 = 250;
/// Quiet time, in milliseconds, before the text is written down as a draft.
pub const DRAFT_DEBOUNCE_MS: u64 = 1500;
/// Quiet time, in milliseconds, after the last keystroke before an agent tab's text goes to the
/// host as a `DbEditorEdit`.
pub const EDITOR_EDIT_DEBOUNCE_MS: u64 = 250;
/// How long, in milliseconds, an edit may go unanswered before it is given up on (the host says
/// nothing to an edit that changes nothing) and the next may go out.
pub const EDITOR_EDIT_LOST_MS: u64 = 2000;

// ── What Run runs ───────────────────────────────────────────────────

/// One statement to run.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub sql: String,
    pub class: StatementClass,
    /// `"SELECT"`, `"UPDATE"`, … — the result tab's label and the confirm bar's words.
    pub kind: String,
    /// Where the statement starts in the editor's text, to point an error at it.
    pub offset: usize,
}

impl Job {
    /// Whether the statement may change data or schema, so it needs a yes first.
    pub fn needs_confirm(&self) -> bool {
        self.class.mutates() || self.class == StatementClass::Other
    }
}

/// Which statements a Run takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The selection if there is one, else the statement under the cursor.
    Statement,
    /// The whole text, statement by statement.
    All,
}

/// The statements `scope` names, with their classes. `selection` and `cursor` are byte offsets
/// into `text`.
pub fn pick(
    text: &str,
    selection: Range<usize>,
    cursor: usize,
    dialect: Dialect,
    scope: Scope,
) -> Vec<Job> {
    let (piece, base): (&str, usize) = match scope {
        Scope::All => (text, 0),
        Scope::Statement if !selection.is_empty() => (
            text.get(selection.clone()).unwrap_or_default(),
            selection.start,
        ),
        Scope::Statement => match sql::statement_at(text, dialect, cursor) {
            Some(range) => (&text[range.clone()], range.start),
            None => return Vec::new(),
        },
    };
    sql::analyze(piece, dialect)
        .statements
        .into_iter()
        .map(|info| Job {
            sql: piece[info.byte_range.clone()].to_string(),
            class: info.class,
            kind: info.kind,
            offset: base + info.byte_range.start,
        })
        .collect()
}

/// What waits for a yes.
#[derive(Clone, Debug, PartialEq)]
pub enum Confirm {
    /// Statements that may change data; whether the run keeps a tab per statement.
    Run { jobs: Vec<Job>, keep_all: bool },
    /// An `EXPLAIN ANALYZE` of a statement that writes — it really executes.
    Analyze(Job),
}

/// The run in flight: what was asked, so a reply can be labelled.
#[derive(Clone, Debug, PartialEq)]
pub struct RunInfo {
    pub jobs: Vec<Job>,
    /// Whether the run keeps one result tab per statement, or only the last.
    pub keep_all: bool,
    /// `Some(analyze)` for an Explain.
    pub explain: Option<bool>,
}

// ── What a run left ─────────────────────────────────────────────────

/// What a result tab shows.
pub enum ResultBody {
    /// A statement with rows. `pending` is the data until the grid is built (it needs a `Window`),
    /// and `None` once it has been.
    Rows {
        pending: Option<ResultSet>,
        grid: Option<Entity<TableState<GridDelegate>>>,
    },
    /// A plan.
    Plan(PlanState),
    /// Nothing to draw beyond the note: an affected count, "Done", or an error.
    Note,
}

/// One result tab.
pub struct SqlResult {
    /// `1 · SELECT`.
    pub label: SharedString,
    /// The line above the body: counts and time, or the error.
    pub note: SharedString,
    pub error: bool,
    pub body: ResultBody,
}

impl SqlResult {
    pub fn is_plan(&self) -> bool {
        matches!(self.body, ResultBody::Plan(_))
    }
}

/// A message with no result behind it ("Nothing to run.").
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub error: bool,
    pub text: SharedString,
}

/// The result tab one reply of a run becomes. `index` is the statement's place in the run.
pub fn result_of(
    run: &RunInfo,
    index: usize,
    outcome: Result<Box<DbOutcome>, DbFailure>,
    elapsed_ms: u64,
    read_only: bool,
) -> SqlResult {
    let total = run.jobs.len();
    let number = index + 1;
    let kind = run.jobs.get(index).map_or("", |job| job.kind.as_str());
    let label = match (run.explain, kind) {
        (Some(_), _) => "Plan".to_string(),
        (None, "") => "error".to_string(),
        _ => format!("{number} · {kind}"),
    };
    // A plain Run of several statements keeps only the last result; say which of them it is.
    let several = !run.keep_all && run.explain.is_none() && total > 1;
    let lead = if several {
        format!("Statement {number} of {total} · ")
    } else {
        String::new()
    };
    let time = format_elapsed(elapsed_ms);
    let (error, note, body) = match outcome {
        Ok(outcome) => match *outcome {
            DbOutcome::Rows(set) if set.columns.is_empty() => {
                (false, format!("{lead}Done · {time}"), ResultBody::Note)
            }
            DbOutcome::Rows(set) => {
                let cut = if set.truncated {
                    " (cut at the row cap)"
                } else {
                    ""
                };
                let note = format!("{lead}{} rows{cut} · {time}", set.rows.len());
                let body = ResultBody::Rows {
                    pending: Some(set),
                    grid: None,
                };
                (false, note, body)
            }
            DbOutcome::Affected(n) => (
                false,
                format!("{lead}{n} rows affected · {time}"),
                ResultBody::Note,
            ),
            DbOutcome::Plan(plan) => {
                let nodes = plan.root.count();
                let what = if run.explain == Some(true) {
                    "Explain analyze"
                } else {
                    "Explain"
                };
                let note = format!(
                    "{what} · {nodes} node{} · {time}",
                    if nodes == 1 { "" } else { "s" }
                );
                (false, note, ResultBody::Plan(PlanState::new(plan)))
            }
        },
        Err(failure) => {
            let message = describe_failure(&failure, elapsed_ms, read_only);
            let note = match (several, run.explain) {
                (true, _) => format!("Statement {number} of {total}: {message}"),
                (_, Some(analyze)) => format!(
                    "{}: {message}",
                    if analyze {
                        "Explain analyze"
                    } else {
                        "Explain"
                    }
                ),
                _ => message,
            };
            (true, note, ResultBody::Note)
        }
    };
    SqlResult {
        label: label.into(),
        note: note.into(),
        error,
        body,
    }
}

// ── The tab ─────────────────────────────────────────────────────────

/// The agent that controls a tab: the host's shared editor it mirrors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentMark {
    /// The editor's name, as the agent gave it.
    pub name: String,
    /// The agent's title, for the chip and the tooltip.
    pub title: String,
}

impl AgentMark {
    /// "Controlled by <title>" — the tab's tooltip.
    pub fn tooltip(&self) -> String {
        format!("Controlled by {}", self.title)
    }

    /// "Agent · <title>" — the chip in the tab header.
    pub fn chip(&self) -> String {
        format!("Agent · {}", self.title)
    }
}

pub struct DbSqlTab {
    pub session: DbSessionId,
    pub conn: DbConnId,
    /// The database the session selects before each run.
    pub database: Option<String>,
    /// The run in flight, which `DbCancel` stops and whose replies this tab accepts.
    pub query: Option<DbQueryId>,
    pub run: Option<RunInfo>,
    pub started: Option<Instant>,
    /// Stop was pressed; the cancel is re-sent while the run goes on, since one that lands between
    /// two statements finds nothing to cancel.
    pub stopping: bool,
    pub stop_ticks: u32,

    /// The editor's seed, then its last known text.
    pub text: String,
    pub editor: Option<Entity<EditorState>>,
    pub highlight: Option<TextDecorationCollection>,
    /// Text length / cursor / selection the highlight was computed for.
    pub highlight_key: Option<(usize, usize, Range<usize>)>,
    /// The statement a plain Run would take, backgrounded in the editor.
    pub current: Option<Range<usize>>,
    pub selecting: bool,
    pub analysis: Analysis,
    pub analysis_gen: u64,
    pub draft_gen: u64,
    /// The toggle. The connection's own read-only is separate and stronger ([`Self::locked`]).
    pub read_only: bool,
    /// Whether the live check allows every statement; `None` when read-only is off or the text
    /// does not parse.
    pub ro_allowed: Option<bool>,
    /// A range an error underlines, and what it says.
    pub mark: Option<(Range<usize>, String)>,

    pub results: Vec<SqlResult>,
    pub active: usize,
    pub notice: Option<Note>,
    pub confirm: Option<Confirm>,
    pub subscriptions: Vec<Subscription>,

    /// Set while an agent controls the tab (a host-side shared editor). Such a tab is never saved
    /// as a draft: the host owns the text.
    pub agent: Option<AgentMark>,
    /// The text the host last held — or last was sent. An edit goes out only when the editor
    /// differs from it, which is what stops the host's echo of our own edit coming back as one.
    pub host_text: String,
    /// The host's revision of the text, sent back as `base_rev`.
    pub rev: u64,
    /// Bumped on each keystroke, so only the last of a burst sends.
    pub edit_gen: u64,
    /// When the `DbEditorEdit` still waiting for its answer went out. Only one is in flight, so a
    /// second never races the first for the revision and gets refused as stale.
    pub edit_in_flight: Option<Instant>,
    /// Text the host wrote that the editor widget has not taken yet (it needs a `Window`).
    pub pending_text: Option<String>,
}

impl DbSqlTab {
    pub fn new(conn: DbConnId, database: Option<String>) -> Self {
        Self::with_session(DbSessionId::generate(), conn, database, String::new())
    }

    /// A tab brought back from a draft, under the session its panel key names.
    pub fn with_session(
        session: DbSessionId,
        conn: DbConnId,
        database: Option<String>,
        text: String,
    ) -> Self {
        Self {
            session,
            conn,
            database,
            query: None,
            run: None,
            started: None,
            stopping: false,
            stop_ticks: 0,
            text,
            editor: None,
            highlight: None,
            highlight_key: None,
            current: None,
            selecting: false,
            analysis: Analysis::default(),
            analysis_gen: 0,
            draft_gen: 0,
            read_only: false,
            ro_allowed: None,
            mark: None,
            results: Vec::new(),
            active: 0,
            notice: None,
            confirm: None,
            subscriptions: Vec::new(),
            agent: None,
            host_text: String::new(),
            rev: 0,
            edit_gen: 0,
            edit_in_flight: None,
            pending_text: None,
        }
    }

    /// A tab for a shared editor the host reported, under the editor's own session.
    pub fn from_editor(editor: &DbEditor) -> Self {
        let mut tab = Self::with_session(
            editor.session,
            editor.conn,
            editor.database.clone(),
            editor.text.clone(),
        );
        tab.agent = Self::mark_of(editor);
        tab.host_text = editor.text.clone();
        tab.rev = editor.rev;
        tab
    }

    fn mark_of(editor: &DbEditor) -> Option<AgentMark> {
        let key = editor.agent_key.as_ref()?;
        Some(AgentMark {
            name: editor.name.clone(),
            title: editor.agent_title.clone().unwrap_or_else(|| key.clone()),
        })
    }

    /// Take what the host now says the editor holds. Returns whether the text must replace the
    /// editor's: the host's own echo of an edit this tab sent changes only the revision. Any answer
    /// ends the edit in flight; text that is not what was sent means an agent wrote, and wins.
    pub fn adopt_editor(&mut self, editor: &DbEditor) -> bool {
        self.agent = Self::mark_of(editor);
        self.database = editor.database.clone();
        self.rev = editor.rev;
        self.edit_in_flight = None;
        if editor.text == self.host_text {
            return false;
        }
        self.host_text = editor.text.clone();
        self.text = editor.text.clone();
        self.pending_text = Some(editor.text.clone());
        true
    }

    /// Whether `text`, as the editor now holds it, is an edit the host has not seen: only an agent
    /// tab sends, only text that differs from what the host holds, and only while no earlier edit
    /// waits for its answer.
    pub fn edit_due(&self, text: &str) -> bool {
        let waiting = self
            .edit_in_flight
            .is_some_and(|at| at.elapsed().as_millis() < u128::from(EDITOR_EDIT_LOST_MS));
        self.agent.is_some() && !waiting && text != self.host_text
    }

    /// Record that `text` is going to the host, so its echo is not mistaken for news.
    pub fn sent_edit(&mut self, text: &str) {
        self.host_text = text.to_string();
        self.edit_in_flight = Some(Instant::now());
    }

    /// The panel's key — [`super::db_sql_key`].
    pub fn key(&self) -> String {
        super::db_sql_key(self.session)
    }

    pub fn is_running(&self) -> bool {
        self.query.is_some()
    }

    /// Whether statements run under the guard: the toggle, or a read-only connection (`locked`).
    pub fn effective_read_only(&self, locked: bool) -> bool {
        self.read_only || locked
    }

    /// `▸ 2/5 UPDATE`: which statement a plain Run takes, or `▸ selection`.
    pub fn run_target(&self) -> Option<String> {
        if self.selecting {
            return Some("▸ selection".into());
        }
        let range = self.current.as_ref()?;
        let statements = &self.analysis.statements;
        let ix = statements
            .iter()
            .position(|s| s.byte_range.start == range.start)?;
        Some(format!(
            "▸ {}/{} {}",
            ix + 1,
            statements.len(),
            statements[ix].kind
        ))
    }
}

// ── Words ───────────────────────────────────────────────────────────

/// `420 ms`, `3.2 s`, `2m 05s`.
pub fn format_elapsed(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1000.0)
    } else {
        let secs = ms / 1000;
        format!("{}m {:02}s", secs / 60, secs % 60)
    }
}

/// Whether a server's own error text is a read-only refusal (Postgres "cannot execute INSERT in a
/// read-only transaction", MySQL "… READ ONLY transaction", SQLite "attempt to write a readonly
/// database").
fn looks_like_refusal(message: &str) -> bool {
    let m = message.to_lowercase();
    m.contains("read-only") || m.contains("read only") || m.contains("readonly")
}

/// The line a failed statement shows. `position` is the server's, a 1-based character offset into
/// the statement, when it gave one.
pub fn describe_failure(failure: &DbFailure, elapsed_ms: u64, read_only: bool) -> String {
    let at = failure
        .position
        .map(|p| format!(" (at character {p})"))
        .unwrap_or_default();
    match failure.kind {
        DbFailureKind::Cancelled => format!("Cancelled after {}", format_elapsed(elapsed_ms)),
        DbFailureKind::Timeout => format!("Timed out after {}", format_elapsed(elapsed_ms)),
        DbFailureKind::ReadOnly => {
            format!("Refused by the read-only guard: {}", failure.message)
        }
        DbFailureKind::Query if read_only && looks_like_refusal(&failure.message) => {
            format!("Refused by the server (read-only): {}{at}", failure.message)
        }
        DbFailureKind::NeedsPassword => {
            format!("This connection needs a password. {}", failure.message)
        }
        _ => format!("{}{at}", failure.message),
    }
}

/// The editor range a failure's position points at: one character at `position` in the statement
/// that starts at `offset` in the text. `None` for a failure with no position or one outside the
/// statement.
pub fn failure_mark(text: &str, job: &Job, failure: &DbFailure) -> Option<Range<usize>> {
    let position = failure.position? as usize;
    let at = position.checked_sub(1)?;
    let (byte, ch) = job.sql.char_indices().nth(at)?;
    let start = job.offset + byte;
    let end = start + ch.len_utf8();
    (end <= text.len() && text.is_char_boundary(start) && text.is_char_boundary(end))
        .then_some(start..end)
}

fn rule_label(rule: ReadOnlyRule) -> &'static str {
    match rule {
        ReadOnlyRule::Unparseable => "unparseable",
        ReadOnlyRule::Empty => "empty",
        ReadOnlyRule::StackedStatements => "stacked statements",
        ReadOnlyRule::StatementKind => "statement kind",
        ReadOnlyRule::NestedDml => "nested DML",
        ReadOnlyRule::SelectInto => "SELECT INTO",
        ReadOnlyRule::Locking => "locking",
        ReadOnlyRule::Function => "function",
        ReadOnlyRule::TableHint => "table hint",
        ReadOnlyRule::OptimizerHint => "optimizer hint",
        ReadOnlyRule::ExecutableComment => "executable comment",
        ReadOnlyRule::Pragma => "pragma",
        ReadOnlyRule::Assignment => "assignment",
    }
}

/// The live read-only check (layer 1, the interface's copy): for each statement `check_read_only`
/// refuses, the range to underline and why. The host runs the same check and its answer is the one
/// that decides.
pub fn read_only_marks(
    text: &str,
    analysis: &Analysis,
    dialect: Dialect,
) -> Vec<(Range<usize>, String)> {
    let mut marks = Vec::new();
    for info in &analysis.statements {
        let Some(piece) = text.get(info.byte_range.clone()) else {
            continue;
        };
        let verdict = sql::check_read_only(piece, dialect);
        for violation in verdict.violations() {
            let range = match &violation.byte_range {
                Some(r) => info.byte_range.start + r.start..info.byte_range.start + r.end,
                None => info.byte_range.clone(),
            };
            let end = range.end.min(text.len());
            let start = range.start.min(end.saturating_sub(1));
            if text.is_char_boundary(start) && text.is_char_boundary(end) && start < end {
                marks.push((
                    start..end,
                    format!(
                        "Not allowed in read-only mode: {} — {}",
                        rule_label(violation.rule),
                        violation.message
                    ),
                ));
            }
        }
    }
    marks
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: Dialect = Dialect::Sqlite;

    #[test]
    fn run_picks_the_statement_under_the_cursor() {
        let text = "select 1;\nselect 2;\nupdate t set a = 1";
        // In the middle of the second statement.
        let cursor = text.find("select 2").unwrap() + 3;
        let jobs = pick(text, cursor..cursor, cursor, D, Scope::Statement);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].sql, "select 2");
        assert_eq!(jobs[0].offset, text.find("select 2").unwrap());

        // In the last one: a write, so it needs a yes.
        let cursor = text.len();
        let jobs = pick(text, cursor..cursor, cursor, D, Scope::Statement);
        assert_eq!(jobs.len(), 1);
        assert!(jobs[0].sql.starts_with("update"));
        assert!(jobs[0].needs_confirm());

        // At the very start: the first.
        let jobs = pick(text, 0..0, 0, D, Scope::Statement);
        assert_eq!(jobs[0].sql, "select 1");
        assert!(!jobs[0].needs_confirm());
    }

    #[test]
    fn a_selection_outranks_the_cursor() {
        let text = "select 1; select 2; select 3";
        let start = text.find("select 2").unwrap();
        let selection = start..text.len();
        let jobs = pick(text, selection, start, D, Scope::Statement);
        let sqls: Vec<_> = jobs.iter().map(|j| j.sql.as_str()).collect();
        assert_eq!(sqls, ["select 2", "select 3"]);
        assert_eq!(jobs[1].offset, text.find("select 3").unwrap());
    }

    #[test]
    fn run_all_takes_every_statement() {
        let text = "select 1;\nselect 2;\ndelete from t";
        let jobs = pick(text, 0..0, 0, D, Scope::All);
        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs[2].kind, "DELETE");
        assert!(pick("   ", 0..0, 0, D, Scope::All).is_empty());
        assert!(pick("   ", 0..0, 0, D, Scope::Statement).is_empty());
    }

    #[test]
    fn elapsed_formats() {
        assert_eq!(format_elapsed(420), "420 ms");
        assert_eq!(format_elapsed(3200), "3.2 s");
        assert_eq!(format_elapsed(125_000), "2m 05s");
    }

    #[test]
    fn failures_are_described() {
        let f = |kind, message: &str| DbFailure::new(kind, message);
        assert_eq!(
            describe_failure(&f(DbFailureKind::Cancelled, ""), 3200, false),
            "Cancelled after 3.2 s"
        );
        assert_eq!(
            describe_failure(&f(DbFailureKind::Timeout, ""), 3200, false),
            "Timed out after 3.2 s"
        );
        assert!(
            describe_failure(&f(DbFailureKind::ReadOnly, "no"), 1, true)
                .starts_with("Refused by the read-only guard")
        );
        let server = f(
            DbFailureKind::Query,
            "cannot execute INSERT in a read-only transaction",
        );
        assert!(describe_failure(&server, 1, true).starts_with("Refused by the server"));
        assert_eq!(describe_failure(&server, 1, false), server.message);
        let mut positioned = f(DbFailureKind::Query, "syntax error");
        positioned.position = Some(8);
        assert_eq!(
            describe_failure(&positioned, 1, false),
            "syntax error (at character 8)"
        );
    }

    #[test]
    fn a_failure_position_points_into_the_statement() {
        let text = "select 1;\nselect * form t";
        let job = pick(text, 0..0, text.len(), D, Scope::Statement).remove(0);
        let mut failure = DbFailure::new(DbFailureKind::Query, "syntax error");
        failure.position = Some(10);
        let range = failure_mark(text, &job, &failure).unwrap();
        assert_eq!(&text[range], "f");
        failure.position = Some(500);
        assert_eq!(failure_mark(text, &job, &failure), None);
        failure.position = None;
        assert_eq!(failure_mark(text, &job, &failure), None);
    }

    #[test]
    fn the_live_check_underlines_what_read_only_refuses() {
        let text = "select 1;\ndelete from t";
        let analysis = sql::analyze(text, D);
        let marks = read_only_marks(text, &analysis, D);
        assert_eq!(marks.len(), 1);
        assert!(marks[0].0.start >= text.find("delete").unwrap());
        assert!(marks[0].1.starts_with("Not allowed in read-only mode"));
        assert!(read_only_marks("select 1", &sql::analyze("select 1", D), D).is_empty());
    }

    #[test]
    fn a_reply_becomes_a_result_tab() {
        let text = "select 1;\nselect 2";
        let jobs = pick(text, 0..0, 0, D, Scope::All);
        let run = RunInfo {
            jobs,
            keep_all: false,
            explain: None,
        };
        let rows = |n: usize, truncated: bool| {
            let meta = ubiq_proto::db::ColumnMeta::new(ubiq_proto::db::DbKind::Sqlite, "a", "INT");
            Ok(Box::new(DbOutcome::Rows(ResultSet {
                columns: vec![meta],
                rows: vec![vec![ubiq_proto::db::Value::Int(1)]; n],
                truncated,
            })))
        };
        let tab = result_of(&run, 1, rows(3, true), 1500, false);
        assert_eq!(tab.label, "2 · SELECT");
        assert_eq!(
            tab.note,
            "Statement 2 of 2 · 3 rows (cut at the row cap) · 1.5 s"
        );
        assert!(!tab.error && matches!(tab.body, ResultBody::Rows { .. }));

        let affected = result_of(&run, 0, Ok(Box::new(DbOutcome::Affected(4))), 20, false);
        assert_eq!(affected.note, "Statement 1 of 2 · 4 rows affected · 20 ms");

        let failed = result_of(
            &run,
            1,
            Err(DbFailure::new(DbFailureKind::Query, "no such table: t")),
            5,
            false,
        );
        assert!(failed.error);
        assert_eq!(failed.note, "Statement 2 of 2: no such table: t");

        let explain = RunInfo {
            jobs: run.jobs.clone(),
            keep_all: false,
            explain: Some(true),
        };
        let plan = ubiq_proto::db::Plan {
            root: ubiq_proto::db::PlanNode::new("SCAN t"),
            raw: String::new(),
            format: ubiq_proto::db::PlanFormat::Table,
        };
        let shown = result_of(&explain, 0, Ok(Box::new(DbOutcome::Plan(plan))), 3, false);
        assert_eq!(shown.label, "Plan");
        assert_eq!(shown.note, "Explain analyze · 1 node · 3 ms");
        assert!(shown.is_plan());
    }

    #[test]
    fn a_tab_names_the_statement_run_would_take() {
        let mut tab = DbSqlTab::new(DbConnId::generate(), None);
        let text = "select 1;\nupdate t set a = 1";
        tab.analysis = sql::analyze(text, D);
        tab.current = sql::statement_at(text, D, text.len());
        assert_eq!(tab.run_target().as_deref(), Some("▸ 2/2 UPDATE"));
        tab.selecting = true;
        assert_eq!(tab.run_target().as_deref(), Some("▸ selection"));
        assert!(tab.effective_read_only(true));
        assert!(!tab.effective_read_only(false));
    }
}
