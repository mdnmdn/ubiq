//! A SQL tab's handlers: opening one, running, stopping, explaining, statements arriving, and the
//! editor's own upkeep (analysis, the statement under the cursor, the draft).
//!
//! Everything is looked up by session: a reply for a tab the user has closed finds nothing and is
//! dropped, and so is one whose `DbQueryId` is not the run the tab is waiting on.
//!
//! **The editor needs a `Window`**, so a tab is born without one and `build_db_sql_widgets` — called
//! from `render` — gives it an editor, then a grid for each result that arrived without one. A tab
//! the dock restored (its panel is in the layout, its state is not) is brought back there too, from
//! `ViewPrefs::db_sql_drafts`.

use std::time::{Duration, Instant};

use gpui_component::highlighter::{Diagnostic, DiagnosticSeverity};
use gpui_component::input::{EditorState, InputEvent, RopeExt as _, TextDecoration};
use gpui_component::table::TableState;
use ubiq_db::sql::{self, Dialect};

use super::*;
use crate::state::db::sql::{
    ANALYSIS_DEBOUNCE_MS, Confirm, DRAFT_DEBOUNCE_MS, Job, Note, ResultBody, RunInfo, Scope,
    failure_mark, pick, read_only_marks, result_of,
};
use crate::state::db::{DbSqlTab, DbState};
use crate::state::prefs::{DB_SQL_DRAFT_MAX, DbSqlDraft};
use crate::theme;
use crate::ui::db::sql::plan::PlanAction;
use crate::ui::db::sql::results::GridDelegate;

/// How often a running statement redraws its timer.
const TICK: Duration = Duration::from_millis(250);
/// Ticks between two re-sent cancels while Stop is pending — a second.
const RECANCEL_TICKS: u32 = 4;

impl AppState {
    /// The SQL tab a panel key names, and the project's database state around it. Looked up across
    /// every project the window holds, so the dock can ask for a tab's label without a context.
    pub fn db_sql_lookup(&self, key: &str) -> Option<(&DbSqlTab, &DbState)> {
        self.projects
            .values()
            .find_map(|open| open.db.sql_tab(key).map(|tab| (tab, &open.db)))
    }

    /// Open a SQL editor tab on a connection, in the bottom region. `database` is the one the
    /// session selects first.
    pub fn open_db_sql(
        &mut self,
        conn: DbConnId,
        database: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.open_db_sql_text(conn, database, String::new(), cx);
    }

    /// [`Self::open_db_sql`] with the editor's first text — an explorer's *Open in SQL editor*
    /// seeds a `SELECT`.
    pub fn open_db_sql_text(
        &mut self,
        conn: DbConnId,
        database: Option<String>,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let mut tab = DbSqlTab::new(conn, database);
        tab.text = text;
        let (key, session) = (tab.key(), tab.session);
        open.db.sqls.push(tab);
        self.pending_panels
            .push(PanelEdit::Open(PanelKind::DbSql(key.clone())));
        self.pending_panels
            .push(PanelEdit::Reveal(PanelKind::DbSql(key)));
        self.save_db_sql_draft(project, session);
        cx.notify();
    }

    // ── Widgets ─────────────────────────────────────────────────────

    /// Build the widgets SQL tabs queued: bring back a restored tab, give a tab its code editor,
    /// and a grid to each result that has none.
    pub(super) fn build_db_sql_widgets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        self.restore_db_sql_tabs(project);
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        let bare: Vec<DbSessionId> = open
            .db
            .sqls
            .iter()
            .filter(|tab| tab.editor.is_none())
            .map(|tab| tab.session)
            .collect();
        for session in bare {
            self.build_db_sql_editor(project, session, window, cx);
        }
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let mut built = false;
        for tab in &mut open.db.sqls {
            for result in &mut tab.results {
                let ResultBody::Rows { pending, grid } = &mut result.body else {
                    continue;
                };
                let Some(set) = pending.take() else {
                    continue;
                };
                *grid = Some(cx.new(|cx| {
                    TableState::new(GridDelegate::new(set), window, cx)
                        .sortable(false)
                        .col_resizable(true)
                }));
                built = true;
            }
        }
        if built {
            cx.notify();
        }
    }

    /// Bring back the tabs the dock's saved layout names and this process no longer holds, from
    /// their drafts. A draft whose connection is gone is dropped; one whose panel is not in the
    /// layout yet is left until it is.
    fn restore_db_sql_tabs(&mut self, project: ProjectId) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        if !open.db.loaded || open.prefs.db_sql_drafts.is_empty() {
            return;
        }
        let mut restored = Vec::new();
        let mut gone = Vec::new();
        for draft in &open.prefs.db_sql_drafts {
            if open.db.sql_tab(&draft.key).is_some() {
                continue;
            }
            let session = db_sql_from_key(&draft.key);
            let conn = draft
                .conn
                .parse::<DbConnId>()
                .ok()
                .filter(|conn| open.db.connection(*conn).is_some());
            let (Some(session), Some(conn)) = (session, conn) else {
                gone.push(draft.key.clone());
                continue;
            };
            if !self
                .panels
                .contains_key(&PanelKind::DbSql(draft.key.clone()))
            {
                continue;
            }
            restored.push(DbSqlTab::with_session(
                session,
                conn,
                draft.database.clone(),
                draft.text.clone(),
            ));
        }
        let changed = !gone.is_empty();
        open.prefs.db_sql_drafts.retain(|d| !gone.contains(&d.key));
        open.db.sqls.extend(restored);
        if changed {
            self.store_prefs(project);
        }
    }

    fn build_db_sql_editor(
        &mut self,
        project: ProjectId,
        session: DbSessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self
            .projects
            .get_mut(&project)
            .and_then(|open| open.db.sql_by_session(session))
        else {
            return;
        };
        let text = tab.text.clone();
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("sql")
                .line_number(true)
                .soft_wrap(false)
                .placeholder(
                    "-- SQL. Run executes the selection or the statement under the cursor.",
                )
                .default_value(text)
        });
        let highlight = editor.update(cx, |editor, cx| {
            editor.create_decorations_collection(Vec::new(), cx)
        });
        let typed = cx.subscribe_in(
            &editor,
            window,
            move |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.db_sql_typed(project, session, cx);
                }
            },
        );
        // The editor notifies on every caret move and edit: that is when the highlight follows.
        let moved = cx.observe(&editor, move |this, _, cx| {
            this.db_sql_moved(project, session, cx)
        });
        if let Some(tab) = self
            .projects
            .get_mut(&project)
            .and_then(|open| open.db.sql_by_session(session))
        {
            tab.editor = Some(editor);
            tab.highlight = Some(highlight);
            tab.subscriptions.extend([typed, moved]);
        }
        self.analyze_db_sql(project, session, cx);
        self.refresh_db_sql_highlight(project, session, cx);
        cx.notify();
    }

    // ── The editor's upkeep ─────────────────────────────────────────

    /// The text changed: an error mark belongs to the text it was made for, so it goes; the
    /// analysis and the draft follow once typing pauses.
    fn db_sql_typed(&mut self, project: ProjectId, session: DbSessionId, cx: &mut Context<Self>) {
        let Some(tab) = self
            .projects
            .get_mut(&project)
            .and_then(|open| open.db.sql_by_session(session))
        else {
            return;
        };
        tab.mark = None;
        tab.analysis_gen = tab.analysis_gen.wrapping_add(1);
        tab.draft_gen = tab.draft_gen.wrapping_add(1);
        let (analysis, draft) = (tab.analysis_gen, tab.draft_gen);
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(ANALYSIS_DEBOUNCE_MS))
                .await;
            let _ = this.update(cx, |this, cx| {
                let current = this
                    .projects
                    .get_mut(&project)
                    .and_then(|open| open.db.sql_by_session(session))
                    .is_some_and(|tab| tab.analysis_gen == analysis);
                if current {
                    this.analyze_db_sql(project, session, cx);
                }
            });
            cx.background_executor()
                .timer(Duration::from_millis(DRAFT_DEBOUNCE_MS))
                .await;
            let _ = this.update(cx, |this, _| {
                let current = this
                    .projects
                    .get_mut(&project)
                    .and_then(|open| open.db.sql_by_session(session))
                    .is_some_and(|tab| tab.draft_gen == draft);
                if current {
                    this.save_db_sql_draft(project, session);
                }
            });
        })
        .detach();
    }

    /// The caret or the selection moved.
    fn db_sql_moved(&mut self, project: ProjectId, session: DbSessionId, cx: &mut Context<Self>) {
        self.refresh_db_sql_highlight(project, session, cx);
    }

    /// Background the statement a plain Run would take. Cheap when nothing moved.
    fn refresh_db_sql_highlight(
        &mut self,
        project: ProjectId,
        session: DbSessionId,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let dialect = dialect_of(open, session);
        let Some(tab) = open.db.sql_by_session(session) else {
            return;
        };
        let (Some(editor), Some(highlight)) = (tab.editor.clone(), tab.highlight.clone()) else {
            return;
        };
        let editor = editor.read(cx);
        let key = (
            editor.text().len(),
            editor.cursor(),
            editor.selected_range(),
        );
        if tab.highlight_key.as_ref() == Some(&key) {
            return;
        }
        let (_, cursor, selection) = key.clone();
        tab.highlight_key = Some(key);
        tab.selecting = !selection.is_empty();
        let text = editor.value();
        // With a selection the selection is what runs, and the editor already paints it.
        tab.current = if tab.selecting {
            None
        } else {
            sql::statement_at(&text, dialect, cursor)
        };
        let style = gpui::HighlightStyle {
            background_color: Some(theme::db_statement_active().into()),
            ..Default::default()
        };
        let decorations = tab
            .current
            .clone()
            .map(|range| TextDecoration::new(range, style))
            .into_iter()
            .collect();
        highlight.set(decorations, cx);
        cx.notify();
    }

    /// Analyse the text as it is: the parse badge, the live read-only verdict, and the editor's
    /// diagnostics — the syntax error, each statement read-only refuses, and the mark a failed run
    /// left.
    fn analyze_db_sql(&mut self, project: ProjectId, session: DbSessionId, cx: &mut Context<Self>) {
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        let dialect = dialect_of_ref(open, session);
        let locked = |tab: &DbSqlTab| open.db.is_read_only(tab.conn);
        let Some(tab) = open.db.sqls.iter().find(|tab| tab.session == session) else {
            return;
        };
        let Some(editor) = tab.editor.clone() else {
            return;
        };
        let ro = tab.effective_read_only(locked(tab));
        let mark = tab.mark.clone();

        let text = editor.read(cx).value();
        let analysis = sql::analyze(&text, dialect);
        let mut marks = Vec::new();
        let mut ro_allowed = None;
        if ro && analysis.error.is_none() && !analysis.statements.is_empty() {
            let refused = read_only_marks(&text, &analysis, dialect);
            ro_allowed = Some(refused.is_empty());
            marks.extend(refused);
        }
        marks.extend(mark);
        let error = analysis.error.clone();
        editor.update(cx, |editor, cx| {
            let rope = editor.text().clone();
            let Some(set) = editor.diagnostics_mut() else {
                return;
            };
            set.reset(&rope);
            if let Some(error) = error {
                // One character at the error; at the very end of the text, the last one.
                let mut start = error.byte_offset.min(text.len());
                if start == text.len() {
                    start = text[..start]
                        .char_indices()
                        .next_back()
                        .map_or(0, |(ix, _)| ix);
                }
                let end = start + text[start..].chars().next().map_or(0, char::len_utf8);
                set.push(
                    Diagnostic::new(
                        rope.offset_to_position(start)..rope.offset_to_position(end),
                        error.message,
                    )
                    .with_severity(DiagnosticSeverity::Error),
                );
            }
            for (range, message) in marks {
                let end = range.end.min(text.len());
                let start = range.start.min(end);
                if start < end && text.is_char_boundary(start) && text.is_char_boundary(end) {
                    set.push(
                        Diagnostic::new(
                            rope.offset_to_position(start)..rope.offset_to_position(end),
                            message,
                        )
                        .with_severity(DiagnosticSeverity::Error),
                    );
                }
            }
            cx.notify();
        });
        if let Some(tab) = self
            .projects
            .get_mut(&project)
            .and_then(|open| open.db.sql_by_session(session))
        {
            tab.analysis = analysis;
            tab.ro_allowed = ro_allowed;
            tab.text = text.to_string();
        }
        cx.notify();
    }

    /// Write the tab's connection, database and text into the project's view blob.
    pub(super) fn save_db_sql_draft(&mut self, project: ProjectId, session: DbSessionId) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(tab) = open.db.sqls.iter().find(|tab| tab.session == session) else {
            return;
        };
        let mut text = tab.text.clone();
        if text.len() > DB_SQL_DRAFT_MAX {
            let mut cut = DB_SQL_DRAFT_MAX;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
        }
        let draft = DbSqlDraft {
            key: tab.key(),
            conn: tab.conn.to_string(),
            database: tab.database.clone(),
            text,
        };
        let drafts = &mut open.prefs.db_sql_drafts;
        match drafts.iter_mut().find(|d| d.key == draft.key) {
            Some(known) if *known == draft => return,
            Some(known) => *known = draft,
            None => drafts.push(draft),
        }
        self.store_prefs(project);
    }

    /// Forget a closed tab's draft.
    pub(super) fn drop_db_sql_draft(&mut self, project: ProjectId, session: DbSessionId) {
        let key = crate::state::db::db_sql_key(session);
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let before = open.prefs.db_sql_drafts.len();
        open.prefs.db_sql_drafts.retain(|d| d.key != key);
        if open.prefs.db_sql_drafts.len() != before {
            self.store_prefs(project);
        }
    }

    // ── Gestures ────────────────────────────────────────────────────

    fn db_sql_tab_mut(
        &mut self,
        session: DbSessionId,
        cx: &Context<Self>,
    ) -> Option<&mut DbSqlTab> {
        let project = self.project(cx)?;
        self.projects.get_mut(&project)?.db.sql_by_session(session)
    }

    fn db_sql_notice(
        &mut self,
        session: DbSessionId,
        error: bool,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.db_sql_tab_mut(session, cx) {
            tab.notice = Some(Note {
                error,
                text: text.to_string().into(),
            });
        }
        cx.notify();
    }

    /// The statements `scope` names, from the editor as it stands.
    fn db_sql_jobs(
        &mut self,
        session: DbSessionId,
        scope: Scope,
        cx: &mut Context<Self>,
    ) -> Vec<Job> {
        let Some(project) = self.project(cx) else {
            return Vec::new();
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return Vec::new();
        };
        let dialect = dialect_of(open, session);
        let Some(editor) = open
            .db
            .sql_by_session(session)
            .and_then(|tab| tab.editor.clone())
        else {
            return Vec::new();
        };
        let editor = editor.read(cx);
        pick(
            &editor.value(),
            editor.selected_range(),
            editor.cursor(),
            dialect,
            scope,
        )
    }

    /// Run: the selection or the statement under the cursor; Run all: every statement, one result
    /// tab each. Statements that may write ask first.
    pub fn db_sql_run(&mut self, session: DbSessionId, scope: Scope, cx: &mut Context<Self>) {
        if self
            .db_sql_tab_mut(session, cx)
            .is_none_or(|tab| tab.is_running())
        {
            return;
        }
        let jobs = self.db_sql_jobs(session, scope, cx);
        if jobs.is_empty() {
            self.db_sql_notice(session, false, "Nothing to run.", cx);
            return;
        }
        let keep_all = scope == Scope::All;
        let (project, locked) = match self.db_sql_read_only(session, cx) {
            Some(found) => found,
            None => return,
        };
        // Under the guard a write is refused by the host, so a yes would only delay the answer.
        let asks = !locked && jobs.iter().any(Job::needs_confirm);
        if asks {
            if let Some(tab) = self.db_sql_tab_mut(session, cx) {
                tab.confirm = Some(Confirm::Run { jobs, keep_all });
                tab.notice = None;
            }
            cx.notify();
        } else {
            self.db_sql_execute(project, session, jobs, keep_all, None, cx);
        }
    }

    /// The active project, and whether the tab runs under the guard.
    fn db_sql_read_only(
        &mut self,
        session: DbSessionId,
        cx: &Context<Self>,
    ) -> Option<(ProjectId, bool)> {
        let project = self.project(cx)?;
        let open = self.projects.get_mut(&project)?;
        let locked_conn = open
            .db
            .sql_by_session(session)
            .map(|tab| tab.conn)
            .map(|conn| open.db.is_read_only(conn))?;
        let tab = open.db.sql_by_session(session)?;
        Some((project, tab.effective_read_only(locked_conn)))
    }

    /// Send the run. `explain` is `Some(analyze)` for a plan.
    fn db_sql_execute(
        &mut self,
        project: ProjectId,
        session: DbSessionId,
        jobs: Vec<Job>,
        keep_all: bool,
        explain: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(conn) = open.db.sql_by_session(session).map(|tab| tab.conn) else {
            return;
        };
        let locked = open.db.is_read_only(conn);
        let Some(tab) = open.db.sql_by_session(session) else {
            return;
        };
        let request = DbQueryRequest {
            conn,
            session,
            database: tab.database.clone(),
            statements: jobs.iter().map(|job| job.sql.clone()).collect(),
            run: match explain {
                Some(analyze) => DbRun::Explain { analyze },
                None => DbRun::Query,
            },
            opts: DbRunOptions {
                read_only: tab.effective_read_only(locked),
                timeout_ms: None,
                row_limit: None,
            },
        };
        tab.confirm = None;
        tab.notice = None;
        tab.run = Some(RunInfo {
            jobs,
            keep_all,
            explain,
        });
        tab.started = Some(Instant::now());
        tab.stopping = false;
        tab.stop_ticks = 0;
        let query = self.run_db_query(project, request);
        if let Some(tab) = self
            .projects
            .get_mut(&project)
            .and_then(|open| open.db.sql_by_session(session))
        {
            tab.query = Some(query);
        }
        self.tick_db_sql(project, session, query, cx);
        cx.notify();
    }

    /// Redraw the running timer, and re-send a pending cancel, until the run ends.
    fn tick_db_sql(
        &mut self,
        project: ProjectId,
        session: DbSessionId,
        query: DbQueryId,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let alive = this.update(cx, |this, cx| {
                    let Some(tab) = this
                        .projects
                        .get_mut(&project)
                        .and_then(|open| open.db.sql_by_session(session))
                        .filter(|tab| tab.query == Some(query))
                    else {
                        return false;
                    };
                    let mut recancel = false;
                    if tab.stopping {
                        tab.stop_ticks += 1;
                        recancel = tab.stop_ticks.is_multiple_of(RECANCEL_TICKS);
                    }
                    if recancel {
                        // A cancel that lands between two statements finds nothing to cancel.
                        this.cancel_db_query(project, session, query);
                    }
                    cx.notify();
                    true
                });
                if !matches!(alive, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Stop the statement that is running.
    pub fn db_sql_stop(&mut self, session: DbSessionId, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(tab) = self.db_sql_tab_mut(session, cx) else {
            return;
        };
        let Some(query) = tab.query else {
            // Nothing is running: the key belongs to whoever is under it.
            cx.propagate();
            return;
        };
        if tab.stopping {
            return;
        }
        tab.stopping = true;
        self.cancel_db_query(project, session, query);
        cx.notify();
    }

    /// Explain the statement Run would take; `analyze` executes it and measures.
    pub fn db_sql_explain(&mut self, session: DbSessionId, analyze: bool, cx: &mut Context<Self>) {
        if self
            .db_sql_tab_mut(session, cx)
            .is_none_or(|tab| tab.is_running())
        {
            return;
        }
        let mut jobs = self.db_sql_jobs(session, Scope::Statement, cx);
        if jobs.len() != 1 {
            let text = if jobs.is_empty() {
                "Nothing to explain."
            } else {
                "Explain takes one statement: select it, or put the cursor in it."
            };
            self.db_sql_notice(session, false, text, cx);
            return;
        }
        let job = jobs.remove(0);
        let Some((project, ro)) = self.db_sql_read_only(session, cx) else {
            return;
        };
        // A plain plan only plans; analyze really executes, so a writing statement asks first.
        if analyze && !ro && job.class.mutates() {
            if let Some(tab) = self.db_sql_tab_mut(session, cx) {
                tab.confirm = Some(Confirm::Analyze(job));
                tab.notice = None;
            }
            cx.notify();
        } else {
            self.db_sql_execute(project, session, vec![job], false, Some(analyze), cx);
        }
    }

    /// The answer to the confirm bar.
    pub fn db_sql_confirm(&mut self, session: DbSessionId, yes: bool, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(confirm) = self
            .db_sql_tab_mut(session, cx)
            .and_then(|tab| tab.confirm.take())
        else {
            return;
        };
        if yes {
            match confirm {
                Confirm::Run { jobs, keep_all } => {
                    self.db_sql_execute(project, session, jobs, keep_all, None, cx)
                }
                Confirm::Analyze(job) => {
                    self.db_sql_execute(project, session, vec![job], false, Some(true), cx)
                }
            }
        }
        cx.notify();
    }

    /// The read-only toggle. A read-only connection is locked on and ignores it.
    pub fn db_sql_toggle_read_only(&mut self, session: DbSessionId, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        if let Some(tab) = self.db_sql_tab_mut(session, cx) {
            tab.read_only = !tab.read_only;
            tab.mark = None;
        }
        self.analyze_db_sql(project, session, cx);
        cx.notify();
    }

    /// Show another result tab.
    pub fn db_sql_select_result(
        &mut self,
        session: DbSessionId,
        ix: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.db_sql_tab_mut(session, cx) {
            tab.active = ix.min(tab.results.len().saturating_sub(1));
            tab.notice = None;
        }
        cx.notify();
    }

    /// A click in a plan view.
    pub fn db_sql_plan(
        &mut self,
        session: DbSessionId,
        result: usize,
        action: PlanAction,
        cx: &mut Context<Self>,
    ) {
        if let Some(ResultBody::Plan(plan)) = self
            .db_sql_tab_mut(session, cx)
            .and_then(|tab| tab.results.get_mut(result))
            .map(|result| &mut result.body)
        {
            plan.apply(action);
        }
        cx.notify();
    }

    // ── Replies ─────────────────────────────────────────────────────

    /// A `DbQueryResult`: one statement's answer.
    pub(super) fn on_db_query_result(
        &mut self,
        project: ProjectId,
        reply: DbQueryReply,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(conn) = open.db.sql_by_session(reply.session).map(|tab| tab.conn) else {
            return;
        };
        let locked = open.db.is_read_only(conn);
        let Some(tab) = open.db.sql_by_session(reply.session) else {
            return;
        };
        // A reply for a run this tab is no longer waiting on is dropped.
        if tab.query != Some(reply.query) {
            return;
        }
        let Some(run) = tab.run.clone() else {
            return;
        };
        let index = reply.index as usize;
        let mark = reply.result.as_ref().err().and_then(|failure| {
            let job = run.jobs.get(index)?;
            let text = &tab.text;
            failure_mark(text, job, failure).map(|range| (range, failure.message.clone()))
        });
        let read_only = tab.effective_read_only(locked);
        let result = result_of(&run, index, reply.result, reply.elapsed_ms, read_only);

        if run.explain.is_some() {
            tab.results.retain(|r| !r.is_plan());
            tab.results.push(result);
            tab.active = tab.results.len() - 1;
        } else if run.keep_all {
            if index == 0 {
                tab.results.clear();
            }
            tab.results.push(result);
        } else {
            tab.results = vec![result];
            tab.active = 0;
        }
        tab.notice = None;
        if reply.last {
            tab.query = None;
            tab.run = None;
            tab.started = None;
            tab.stopping = false;
            // The first failure if there is one, else the first result.
            if run.explain.is_none() {
                tab.active = tab.results.iter().position(|r| r.error).unwrap_or(0);
            }
        }
        let marked = mark.is_some();
        if marked {
            tab.mark = mark;
        }
        if marked {
            self.analyze_db_sql(project, reply.session, cx);
        }
        cx.notify();
    }
}

/// The dialect of the connection a tab is on; `Generic` when the connection is gone.
fn dialect_of(open: &mut crate::app::OpenProject, session: DbSessionId) -> Dialect {
    let conn = open.db.sql_by_session(session).map(|tab| tab.conn);
    conn.and_then(|conn| open.db.connection(conn))
        .map_or(Dialect::Generic, |c| Dialect::from(c.config.kind))
}

fn dialect_of_ref(open: &crate::app::OpenProject, session: DbSessionId) -> Dialect {
    open.db
        .sqls
        .iter()
        .find(|tab| tab.session == session)
        .and_then(|tab| open.db.connection(tab.conn))
        .map_or(Dialect::Generic, |c| Dialect::from(c.config.kind))
}
