//! A SQL tab — a bottom-dock panel, several at once.
//!
//! The code editor with the `sql` grammar over a results area, under one toolbar. **What Run would
//! do is visible**: the statement under the cursor carries the `db_statement_active` ground (an
//! editor text decoration, kept by `AppState::db_sql_moved`) and the toolbar names it (`▸ 2/5
//! UPDATE`); with a selection, the selection is the highlight. A syntax error is an editor
//! diagnostic, so is each statement the read-only check refuses.
//!
//! **Read-only is evident**: the toolbar takes the `db_read_only_soft` ground, a strip in
//! `db_read_only` sits over the editor, and a READ-ONLY chip names it. A read-only connection
//! locks the toggle on. The check here is the live copy; the host's refusal is what decides.
//!
//! The results area keeps one tab per statement of a Run all (`1 · SELECT`, `2 · UPDATE`…), or the
//! last result of a plain Run, and a "Plan" tab for Explain. The grid is [`results`], the plan view
//! [`plan`]; the state they read is `state/db/sql.rs`, the handlers `app/db/sql.rs`.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _,
    Window, div, px, relative,
};
use gpui_component::input::Editor;
use gpui_component::table::DataTable;
use gpui_component::tooltip::Tooltip;
use gpui_component::{Sizable as _, Size, resizable_panel, v_resizable};

use crate::app::AppState;
use crate::state::db::sql::{Confirm, DbSqlTab, ResultBody, Scope};
use crate::theme::{self, Family, Role};
use crate::ui::dock::TabInfo;
use crate::ui::kit::{choice_pill, ghost_button, primary_button, state_chip};
use crate::ui::viewer::note;
use ubiq_db::sql::StatementClass;

use super::keys::{DbRunAll, DbRunStatement, DbStop, SQL_CONTEXT};

#[path = "plan.rs"]
pub mod plan;
#[path = "results.rs"]
pub mod results;

/// The modifier the toolbar's hints name.
const MOD: &str = if cfg!(target_os = "macos") {
    "⌘"
} else {
    "Ctrl+"
};

/// The panel body for the SQL tab named by `key`.
pub fn render(app: &AppState, key: &str, cx: &mut Context<AppState>) -> AnyElement {
    let Some((tab, db)) = app.db_sql_lookup(key) else {
        return note("This SQL tab is closed.", theme::text_faint());
    };
    let session = tab.session;
    let locked = db.is_read_only(tab.conn);
    let ro = tab.effective_read_only(locked);
    let label = connection_label(tab, db.connection(tab.conn).map(|c| c.config.name.as_str()));
    let confirm = tab.confirm.clone();

    div()
        .key_context(SQL_CONTEXT)
        .on_action(cx.listener(move |this, _: &DbRunStatement, _, cx| {
            this.db_sql_run(session, Scope::Statement, cx)
        }))
        .on_action(
            cx.listener(move |this, _: &DbRunAll, _, cx| this.db_sql_run(session, Scope::All, cx)),
        )
        .on_action(cx.listener(move |this, _: &DbStop, _, cx| this.db_sql_stop(session, cx)))
        .flex()
        .flex_col()
        .size_full()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::pane_bg())
        .child(toolbar(tab, label, locked, ro, cx))
        .when_some(confirm, |this, confirm| {
            this.child(confirm_bar(session, &confirm, cx))
        })
        .child(
            div().flex_1().min_h(px(0.)).child(
                v_resizable(crate::ui::eid("db-sql-split", session))
                    .child(
                        resizable_panel()
                            .size(px(theme::scaled(180.0)))
                            .size_range(px(theme::scaled(60.0))..gpui::Pixels::MAX)
                            .child(editor_pane(tab, ro)),
                    )
                    .child(resizable_panel().child(results_area(tab, cx))),
            ),
        )
        .into_any_element()
}

/// The tab's label, tooltip and marks (a pulse while a statement runs).
pub fn tab(app: &AppState, key: &str) -> TabInfo {
    let Some((tab, db)) = app.db_sql_lookup(key) else {
        return TabInfo {
            label: "SQL".into(),
            ..TabInfo::default()
        };
    };
    let name = db.connection(tab.conn).map(|c| c.config.name.as_str());
    let locked = db.is_read_only(tab.conn);
    let (dot_colour, dot_pulse) = if tab.is_running() {
        (Some(theme::info()), true)
    } else if tab.results.iter().any(|r| r.error) {
        (Some(theme::danger()), false)
    } else if tab.effective_read_only(locked) {
        (Some(theme::db_read_only()), false)
    } else {
        (None, false)
    };
    TabInfo {
        label: format!("SQL · {}", name.unwrap_or("?")).into(),
        dot_colour,
        dot_pulse,
        tooltip: Some(connection_label(tab, name).into()),
        ..TabInfo::default()
    }
}

/// `orders (staging) · orders`.
fn connection_label(tab: &DbSqlTab, name: Option<&str>) -> String {
    match (name, tab.database.as_deref()) {
        (Some(name), Some(database)) => format!("{name} · {database}"),
        (Some(name), None) => name.to_string(),
        (None, _) => "connection removed".to_string(),
    }
}

/// A click handler over the window, in the shape the kit's buttons take.
fn click(
    cx: &Context<AppState>,
    f: impl Fn(&mut AppState, &mut Context<AppState>) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let view = cx.entity();
    move |_, _, cx| {
        view.update(cx, |this, cx| f(this, cx));
    }
}

/// A toolbar button that is greyed out when it cannot be pressed. A press is also refused where
/// the handler runs, so this is only the look.
fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    tip: impl Into<SharedString>,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<gpui::Div> {
    let tip: SharedString = tip.into();
    ghost_button(id, None, label, on_click)
        .when(!enabled, |this| this.opacity(0.4).cursor_default())
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
}

fn toolbar(
    tab: &DbSqlTab,
    label: String,
    locked: bool,
    ro: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let session = tab.session;
    let idle = !tab.is_running();
    let (badge, badge_colour) = badge(tab);
    let micro = theme::font(Family::Chrome, Role::Meta);

    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap_1()
        .px_2()
        .h(px(theme::scaled(36.0)))
        .bg(if ro {
            theme::db_read_only_soft()
        } else {
            theme::surface()
        })
        .border_b(px(theme::hairline()))
        .border_color(if ro {
            theme::db_read_only()
        } else {
            theme::border()
        })
        .child(
            div()
                .flex_none()
                .max_w(px(theme::scaled(280.0)))
                .truncate()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text())
                .child(SharedString::from(label)),
        )
        .child(div().w(px(theme::hairline())).h_4().bg(theme::border()))
        .child(button(
            "db-sql-run",
            format!("Run {MOD}↵"),
            "Run the selection, or the statement under the cursor",
            idle,
            click(cx, move |this, cx| {
                this.db_sql_run(session, Scope::Statement, cx)
            }),
        ))
        .child(button(
            "db-sql-run-all",
            "Run all",
            format!("Run every statement, one result tab each ({MOD}⇧↵)"),
            idle,
            click(cx, move |this, cx| this.db_sql_run(session, Scope::All, cx)),
        ))
        .child(button(
            "db-sql-explain",
            "Explain",
            "Show the plan of the statement Run would take",
            idle,
            click(cx, move |this, cx| this.db_sql_explain(session, false, cx)),
        ))
        .child(button(
            "db-sql-analyze",
            "Analyze",
            "Explain analyze: executes the statement and measures it",
            idle,
            click(cx, move |this, cx| this.db_sql_explain(session, true, cx)),
        ))
        .when_some(tab.run_target(), |this, target| {
            this.child(
                div()
                    .flex_none()
                    .text_size(micro)
                    .text_color(theme::text_faint())
                    .child(SharedString::from(target)),
            )
        })
        .when(tab.is_running(), |this| {
            let elapsed = tab
                .started
                .map(|at| crate::state::db::sql::format_elapsed(at.elapsed().as_millis() as u64))
                .unwrap_or_default();
            let word = if tab.stopping { "Stopping" } else { "Running" };
            this.child(
                div()
                    .flex_none()
                    .px_1()
                    .text_size(micro)
                    .text_color(theme::info())
                    .child(SharedString::from(format!("{word} {elapsed}"))),
            )
            .child(
                button(
                    "db-sql-stop",
                    "Stop",
                    format!("Cancel the running statement ({MOD}.)"),
                    !tab.stopping,
                    click(cx, move |this, cx| this.db_sql_stop(session, cx)),
                )
                .text_color(theme::danger()),
            )
        })
        .child(div().flex_1())
        .child(if locked {
            state_chip("Read-only (connection)", theme::db_read_only(), 1.0).into_any_element()
        } else {
            choice_pill(
                "db-sql-ro",
                "Read-only",
                tab.read_only,
                click(cx, move |this, cx| {
                    this.db_sql_toggle_read_only(session, cx)
                }),
            )
            .into_any_element()
        })
        .when(ro, |this| {
            this.when_some(tab.ro_allowed, |this, allowed| {
                this.child(
                    div()
                        .flex_none()
                        .text_size(micro)
                        .text_color(if allowed {
                            theme::success()
                        } else {
                            theme::danger()
                        })
                        .child(if allowed {
                            "read-only ✓"
                        } else {
                            "read-only ✗"
                        }),
                )
            })
        })
        .child(
            div()
                .flex_shrink(1.)
                .min_w(px(0.))
                .max_w(px(theme::scaled(420.0)))
                .overflow_hidden()
                .child(state_chip(badge, badge_colour, 1.0)),
        )
        .into_any_element()
}

/// The parse badge: what the text is, from the latest analysis.
fn badge(tab: &DbSqlTab) -> (SharedString, gpui::Rgba) {
    if let Some(error) = &tab.analysis.error {
        let text = format!(
            "Syntax error {}:{} {}",
            error.line, error.column, error.message
        );
        return (text.into(), theme::danger());
    }
    let statements = &tab.analysis.statements;
    if statements.is_empty() {
        return ("Empty".into(), theme::text_faint());
    }
    let any = |class: StatementClass| statements.iter().any(|s| s.class == class);
    if any(StatementClass::Ddl) {
        ("DDL".into(), theme::warning())
    } else if any(StatementClass::Write) {
        ("Write".into(), theme::warning())
    } else if tab.analysis.is_read_only() {
        ("Read".into(), theme::success())
    } else {
        ("Other".into(), theme::text_muted())
    }
}

/// The inline question for statements that change data or schema.
fn confirm_bar(
    session: ubiq_proto::ids::DbSessionId,
    confirm: &Confirm,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let text = match confirm {
        Confirm::Run { jobs, .. } => {
            let mut kinds: Vec<&str> = jobs
                .iter()
                .filter(|job| job.needs_confirm())
                .map(|job| job.kind.as_str())
                .collect();
            kinds.dedup();
            format!("{} may change data or schema. Run?", kinds.join(", "))
        }
        Confirm::Analyze(job) => {
            format!("Explain analyze executes the {} for real. Run?", job.kind)
        }
    };
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .bg(theme::surface_raised())
        .border_b(px(theme::hairline()))
        .border_color(theme::warning())
        .text_size(theme::font(Family::Chrome, Role::Body))
        .child(
            div()
                .text_color(theme::warning())
                .child(SharedString::from(text)),
        )
        .child(primary_button(
            "db-sql-confirm-run",
            None,
            "Run anyway",
            click(cx, move |this, cx| this.db_sql_confirm(session, true, cx)),
        ))
        .child(ghost_button(
            "db-sql-confirm-cancel",
            None,
            "Cancel",
            click(cx, move |this, cx| this.db_sql_confirm(session, false, cx)),
        ))
        .into_any_element()
}

/// The code editor, with a strip over it while the tab cannot write.
fn editor_pane(tab: &DbSqlTab, ro: bool) -> AnyElement {
    let body = match &tab.editor {
        Some(editor) => Editor::new(editor)
            .h(relative(1.))
            .p_0()
            .border_0()
            .text_size(theme::font(Family::Content, Role::Body))
            .into_any_element(),
        None => note("\u{2026}", theme::text_faint()),
    };
    div()
        .flex()
        .flex_col()
        .size_full()
        .when(ro, |this| {
            this.child(
                div()
                    .flex_none()
                    .h(px(theme::scaled(3.0)))
                    .w_full()
                    .bg(theme::db_read_only()),
            )
        })
        .child(div().flex_1().min_h(px(0.)).child(body))
        .into_any_element()
}

/// The result tabs, the line about the active one, and its body.
fn results_area(tab: &DbSqlTab, cx: &mut Context<AppState>) -> AnyElement {
    let session = tab.session;
    let active = tab.results.get(tab.active);
    let body = match active.map(|r| &r.body) {
        Some(ResultBody::Rows {
            grid: Some(grid), ..
        }) => div()
            .size_full()
            .child(
                DataTable::new(grid)
                    .with_size(Size::Medium)
                    .stripe(true)
                    .bordered(false),
            )
            .into_any_element(),
        Some(ResultBody::Plan(state)) => plan::view(session, tab.active, state, cx),
        Some(_) => div().into_any_element(),
        None if tab.notice.is_none() => {
            note("Run a statement to see its rows here.", theme::text_faint())
        }
        None => div().into_any_element(),
    };
    // A message about the last attempt ("Nothing to run.") outranks the note of the result behind it.
    let line = tab
        .notice
        .as_ref()
        .map(|n| (n.error, n.text.clone()))
        .or_else(|| active.map(|r| (r.error, r.note.clone())));

    div()
        .flex()
        .flex_col()
        .size_full()
        .min_h(px(0.))
        .when(tab.results.len() > 1, |this| {
            this.child(
                div()
                    .id(crate::ui::eid("db-sql-result-tabs", session))
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .overflow_x_scroll()
                    .border_b(px(theme::hairline()))
                    .border_color(theme::border())
                    .children(tab.results.iter().enumerate().map(|(ix, result)| {
                        let pill = choice_pill(
                            crate::ui::eid2("db-sql-result", session, ix),
                            result.label.clone(),
                            ix == tab.active,
                            click(cx, move |this, cx| {
                                this.db_sql_select_result(session, ix, cx)
                            }),
                        );
                        if result.error {
                            pill.text_color(theme::danger())
                        } else {
                            pill
                        }
                    })),
            )
        })
        .when_some(line, |this, (error, text)| {
            this.child(
                div()
                    .id(crate::ui::eid("db-sql-note", session))
                    .flex_none()
                    .max_h(px(theme::scaled(120.0)))
                    .overflow_y_scroll()
                    .px_2()
                    .py_1()
                    .border_b(px(theme::hairline()))
                    .border_color(theme::border())
                    .text_size(theme::font(Family::Chrome, Role::Body))
                    .text_color(if error {
                        theme::danger()
                    } else {
                        theme::text_muted()
                    })
                    .child(text),
            )
        })
        .child(div().flex_1().min_h(px(0.)).child(body))
        .into_any_element()
}
