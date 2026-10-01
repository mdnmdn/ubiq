//! A table tab — an editor-like centre panel for one table.
//!
//! A toolbar with the WHERE and ORDER BY fields (validated as typed by `edit::validate_fragment`),
//! Refresh, paging and the count; the edit bar — the lock, `+ Row`, Delete, Discard, Apply, the
//! Inline | Form switch, the SQL preview; the grid ([`grid`]) and, in Form mode or for a read-only
//! tab's inspector, the side form of the selected row; and the JSON dialog ([`json`]) painted over
//! the whole tab while one is open.
//!
//! **Editing: inline cells or a side form, over one pending state.** Inline: double-click (or
//! Enter / F2) on a cell edits it in place and the side form is hidden. Form: the grid is
//! selection-only and selecting a row shows a panel with one [`cell_input`] per column. Every
//! keystroke in the form is validated with `parse_input` against the column and, if valid,
//! becomes a pending value in the grid's overlay (the cell turns bold, the row tinted). An invalid
//! text is shown red under its field and is *not* pending.
//!
//! **Read-only.** The tab is read-only when its connection is, for a view or synonym, and when its
//! lock is on. Then the grid does not edit, the form is a row inspector, and `+ Row` / Delete /
//! Apply are hidden. The host enforces it as well (`DbTablePage.read_only`).
//!
//! This file draws; every gesture is a method of [`AppState`] in `app/db/table.rs`, and every
//! widget this tab needs a `Window` for is built there too, into [`TableWidgets`].

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, ElementId, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, px,
};
use gpui_component::input::{Input, InputState};
use gpui_component::{IconName, Sizable as _};
use ubiq_db::edit::{has_unique_key, render as render_edit};
use ubiq_proto::db::DbKind;

use crate::app::AppState;
use crate::state::db::pending::RowMark;
use crate::state::db::table::{DbTableTab, EditMode};
use crate::state::db::{DbState, db_tab_label};
use crate::theme::{self, Family, Role};
use crate::ui::dock::TabInfo;
use crate::ui::kit::{
    choice_pill, field, ghost_button, icon_button_tip, primary_button, section_label,
};
use crate::ui::viewer::note;

#[path = "cell_input.rs"]
pub mod cell_input;
#[path = "grid.rs"]
pub mod grid;
#[path = "json.rs"]
pub mod json;

use cell_input::CellInput;
use grid::ResultGrid;
use json::{JsonEditor, open_button};

use super::keys::{DbEditCell, TABLE_CONTEXT};

/// Width of the row form at `ui_scale = 1.0`.
const FORM_WIDTH: f32 = 300.0;
/// A toolbar row's height at `ui_scale = 1.0`: the icon buttons' own.
const BAR_HEIGHT: f32 = 30.0;
/// The SQL preview's height at `ui_scale = 1.0`.
const PREVIEW_HEIGHT: f32 = 150.0;
/// The note strip's tallest, before it scrolls.
const NOTE_MAX_HEIGHT: f32 = 120.0;

/// One input of the row form, with the subscription that listens to it.
pub struct FormField {
    pub input: Entity<CellInput>,
    pub _subscription: Subscription,
}

/// The open JSON dialog.
pub struct JsonOpen {
    pub dialog: Entity<JsonEditor>,
    pub _subscription: Subscription,
}

/// The widgets one table tab owns. They need a `Window` to build, so the tab's state holds them as
/// an opaque box (`DbTableTab::widgets`) and `app/db/table.rs` builds them in `render`'s pass.
pub struct TableWidgets {
    pub grid: Entity<ResultGrid>,
    pub where_input: Entity<InputState>,
    pub order_input: Entity<InputState>,
    pub fields: Vec<FormField>,
    pub json: Option<JsonOpen>,
    pub _subscriptions: Vec<Subscription>,
}

impl TableWidgets {
    pub fn of(tab: &DbTableTab) -> Option<&Self> {
        tab.widgets.as_ref()?.downcast_ref()
    }

    pub fn of_mut(tab: &mut DbTableTab) -> Option<&mut Self> {
        tab.widgets.as_mut()?.downcast_mut()
    }
}

// ── The tab strip ───────────────────────────────────────────────────

/// The tab's label, tooltip and marks (a dirty dot while edits are pending).
pub fn tab(app: &AppState, key: &str, cx: &App) -> TabInfo {
    match app.db(cx).and_then(|db| db.table_tab(key)) {
        Some(tab) => TabInfo {
            label: tab.table.name.clone().into(),
            tooltip: Some(SharedString::from(tab.table.to_string())),
            dot_colour: (tab.pending_rows > 0).then(theme::warning),
            ..TabInfo::default()
        },
        None => TabInfo {
            label: db_tab_label(key).into(),
            ..TabInfo::default()
        },
    }
}

// ── Shared pieces (the grid, the editors and the dialog use them too) ──

/// A read-only tag in the lock colour on its tint: in a toolbar, and in a dialog opened on a cell
/// that cannot be written.
pub fn read_only_badge(label: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .px_1p5()
        .bg(theme::db_read_only_soft())
        .border(px(theme::hairline()))
        .border_color(theme::db_read_only())
        .text_size(theme::font(Family::Chrome, Role::Meta))
        .text_color(theme::db_read_only())
        .child(label.into())
}

/// A dialog button: filled when `primary`, outlined otherwise, drained when not `enabled`. Square,
/// like everything else.
pub fn button(
    id: &'static str,
    label: &'static str,
    primary: bool,
    enabled: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .px_3()
        .py_1()
        .text_size(theme::font(Family::Chrome, Role::Body))
        .when(enabled, |this| this.cursor_pointer())
        .when(!enabled, |this| this.opacity(0.4))
        .when(primary, |this| {
            this.bg(theme::accent())
                .text_color(theme::on_accent())
                .when(enabled, |this| {
                    this.hover(|style| style.bg(theme::accent_muted()))
                })
        })
        .when(!primary, |this| {
            this.border(px(theme::hairline()))
                .border_color(theme::border())
                .text_color(theme::text())
                .when(enabled, |this| this.hover(|style| style.bg(theme::hover())))
        })
        .child(label)
}

// ── The body ────────────────────────────────────────────────────────

/// The panel body for the table tab named by `key`.
pub fn render(app: &AppState, key: &str, cx: &mut Context<AppState>) -> AnyElement {
    let Some(db) = app.db(cx) else {
        return note("No project", theme::text_faint());
    };
    let Some(tab) = db.table_tab(key) else {
        return note(db_tab_label(key), theme::text_faint());
    };
    let Some(widgets) = TableWidgets::of(tab) else {
        return note(format!("Opening {}…", tab.table.name), theme::text_faint());
    };
    let view = View::new(db, tab, widgets, key);

    let filter_bar = filter_bar(&view, cx);
    let edit_bar = edit_bar(&view, cx);
    let body = div()
        .flex()
        .flex_row()
        .flex_1()
        .min_h(px(0.))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .h_full()
                .child(widgets.grid.clone()),
        )
        .when(tab.form_visible(view.conn_ro), |this| {
            this.child(row_form(&view, cx))
        });

    let note_strip = tab.note.as_ref().map(|note| {
        strip(
            "db-tbl-note",
            if note.error {
                theme::danger()
            } else {
                theme::text_muted()
            },
            note.text.clone(),
        )
        .max_h(px(theme::scaled(NOTE_MAX_HEIGHT)))
        .overflow_y_scroll()
    });

    let edit_key = key.to_string();
    div()
        .id(ElementId::Name(format!("db-table:{key}").into()))
        .key_context(TABLE_CONTEXT)
        .on_action(cx.listener(move |this, _: &DbEditCell, window, cx| {
            this.db_table_edit_cell(&edit_key, window, cx)
        }))
        .relative()
        .flex()
        .flex_col()
        .size_full()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::pane_bg())
        .child(filter_bar)
        .when_some(tab.fragment_error.clone(), |this, error| {
            this.child(strip("db-tbl-fragment", theme::danger(), error))
        })
        .child(edit_bar)
        .children(note_strip)
        .child(body)
        .when(view.can_edit && tab.show_sql, |this| {
            this.child(preview(&view, cx))
        })
        // The JSON dialog covers the whole tab.
        .when_some(widgets.json.as_ref(), |this, json| {
            this.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .child(json.dialog.clone()),
            )
        })
        .into_any_element()
}

/// What the drawing functions read, gathered once.
struct View<'a> {
    key: &'a str,
    tab: &'a DbTableTab,
    widgets: &'a TableWidgets,
    kind: DbKind,
    conn_ro: bool,
    can_edit: bool,
    /// Rows of the page with anything pending.
    pending: usize,
}

impl<'a> View<'a> {
    fn new(
        db: &'a DbState,
        tab: &'a DbTableTab,
        widgets: &'a TableWidgets,
        key: &'a str,
    ) -> Self {
        let conn_ro = db.is_read_only(tab.conn);
        Self {
            key,
            tab,
            widgets,
            kind: db
                .connection(tab.conn)
                .map_or(DbKind::Postgres, |conn| conn.config.kind),
            conn_ro,
            can_edit: tab.can_edit(conn_ro),
            pending: tab.pending_rows,
        }
    }
}

/// A click handler that calls `f` on the root view with this tab's key.
fn click<F>(
    cx: &mut Context<AppState>,
    key: &str,
    f: F,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static
where
    F: Fn(&mut AppState, &str, &mut Window, &mut Context<AppState>) + 'static,
{
    let key = key.to_string();
    cx.listener(move |this, _: &ClickEvent, window, cx| f(this, &key, window, cx))
}

fn eid(tag: &'static str, key: &str) -> ElementId {
    ElementId::Name(format!("{tag}:{key}").into())
}

/// A message strip under the bars.
fn strip(
    id: &'static str,
    colour: gpui::Rgba,
    text: impl Into<SharedString>,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px_2()
        .py_1()
        .border_b(px(theme::hairline()))
        .border_color(theme::border())
        .text_size(theme::font(Family::Chrome, Role::Label))
        .text_color(colour)
        .child(text.into())
}

/// A toolbar row: flush, full height, a rule under it.
fn bar() -> Div {
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap_1()
        .pl_2()
        .h(px(theme::scaled(BAR_HEIGHT)))
        .bg(theme::surface())
        .border_b(px(theme::hairline()))
        .border_color(theme::border())
        .text_size(theme::font(Family::Chrome, Role::Label))
}

fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .text_color(theme::text_muted())
        .child(text.into())
}

/// A bar button: a ghost button with its handler when it can be pressed, faint text when it cannot.
fn action(
    id: ElementId,
    icon: Option<IconName>,
    label: impl Into<SharedString>,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    if enabled {
        ghost_button(id, icon, label, on_click).into_any_element()
    } else {
        div()
            .id(id)
            .px_2()
            .text_color(theme::text_faint())
            .child(label.into())
            .into_any_element()
    }
}

/// A labelled text field of the toolbar: the label, then the field in the kit's shape.
fn input_box(label: &'static str, input: &Entity<InputState>) -> Div {
    div()
        .flex()
        .flex_row()
        .flex_1()
        .min_w(px(theme::scaled(120.0)))
        .h_full()
        .items_center()
        .gap_1()
        .child(muted(label))
        .child(
            field(theme::border(), false)
                .flex_1()
                .h_full()
                .min_w(px(0.))
                .px_2()
                .child(Input::new(input).appearance(false).small()),
        )
}

fn filter_bar(view: &View, cx: &mut Context<AppState>) -> Div {
    let tab = view.tab;
    let key = view.key;
    let busy = tab.loading;
    bar()
        .child(input_box("WHERE", &view.widgets.where_input))
        .child(input_box("ORDER BY", &view.widgets.order_input))
        .child(action(
            eid("db-tbl-filter", key),
            None,
            "Filter",
            !busy,
            click(cx, key, |this, key, _, cx| {
                this.db_table_apply_fragments(key, cx)
            }),
        ))
        .child(icon_button_tip(
            eid("db-tbl-refresh", key),
            IconName::RotateCw,
            "Refresh the page and its count",
            !busy,
            click(cx, key, |this, key, _, cx| this.db_table_refresh(key, cx)),
        ))
        .child(action(
            eid("db-tbl-size", key),
            None,
            format!("{} / page", tab.page_size),
            !busy,
            click(cx, key, |this, key, _, cx| {
                this.db_table_next_page_size(key, cx)
            }),
        ))
        .child(icon_button_tip(
            eid("db-tbl-prev", key),
            IconName::ChevronLeft,
            "Previous page",
            tab.can_prev(),
            click(cx, key, |this, key, _, cx| {
                this.db_table_turn_page(key, false, cx)
            }),
        ))
        .child(muted(tab.count_label()))
        .child(icon_button_tip(
            eid("db-tbl-next", key),
            IconName::ChevronRight,
            "Next page",
            tab.can_next(),
            click(cx, key, |this, key, _, cx| {
                this.db_table_turn_page(key, true, cx)
            }),
        ))
}

fn edit_bar(view: &View, cx: &mut Context<AppState>) -> Div {
    let tab = view.tab;
    let key = view.key;
    let any_error = tab.field_errors.iter().any(Option::is_some);
    let has_row = tab.selected.is_some();
    let ready = !tab.loading && !tab.applying;
    let no_key = !tab.columns.is_empty() && !has_unique_key(&tab.columns);
    let read_only = tab.read_only(view.conn_ro);

    // The lock leads the bar. Forced: a plain tag that says why. Otherwise a toggle.
    let lock = if tab.forced(view.conn_ro) {
        let why = if tab.editable {
            "The connection is read-only"
        } else {
            "A view or synonym is always read-only"
        };
        div()
            .id(eid("db-tbl-lock", key))
            .flex_none()
            .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(why).build(window, cx)
            })
            .child(read_only_badge("READ-ONLY"))
            .into_any_element()
    } else {
        choice_pill(
            eid("db-tbl-lock", key),
            "Read-only",
            tab.locked,
            click(cx, key, |this, key, window, cx| {
                this.db_table_toggle_lock(key, window, cx)
            }),
        )
        .tooltip(|window, cx| {
            gpui_component::tooltip::Tooltip::new(
                "Read-only: no editing, queries run under the read-only guard",
            )
            .build(window, cx)
        })
        .into_any_element()
    };

    let mut bar = bar().child(lock);
    if read_only && !tab.forced(view.conn_ro) {
        bar = bar.bg(theme::db_read_only_soft()).child(read_only_badge("READ-ONLY"));
    } else if read_only {
        bar = bar.bg(theme::db_read_only_soft());
    }

    if view.can_edit {
        bar = bar
            .child(icon_button_tip(
                eid("db-tbl-add", key),
                IconName::Plus,
                "Add a row",
                ready && !tab.columns.is_empty(),
                click(cx, key, |this, key, window, cx| {
                    this.db_table_add_row(key, window, cx)
                }),
            ))
            .child(icon_button_tip(
                eid("db-tbl-delete", key),
                IconName::Delete,
                "Mark the selected row for deletion (again to take it back)",
                ready && has_row,
                click(cx, key, |this, key, window, cx| {
                    this.db_table_delete_row(key, window, cx)
                }),
            ))
            .child(icon_button_tip(
                eid("db-tbl-discard", key),
                IconName::Undo,
                "Discard every pending change",
                ready && view.pending > 0,
                click(cx, key, |this, key, window, cx| {
                    this.db_table_discard(key, window, cx)
                }),
            ));
        let can_apply = ready && view.pending > 0 && !any_error;
        bar = bar.child(if can_apply {
            primary_button(
                eid("db-tbl-apply", key),
                Some(IconName::Check),
                "Apply",
                click(cx, key, |this, key, _, cx| this.db_table_apply(key, cx)),
            )
            .into_any_element()
        } else {
            div()
                .id(eid("db-tbl-apply", key))
                .px_2()
                .text_color(theme::text_faint())
                .child("Apply")
                .into_any_element()
        });
        // Inline | Form: where this tab edits.
        let segment = |tag: &'static str, label: &'static str, mode: EditMode, cx: &mut Context<AppState>| {
            choice_pill(
                eid(tag, key),
                label,
                tab.edit_mode == mode,
                click(cx, key, move |this, key, window, cx| {
                    this.db_table_set_edit_mode(key, mode, window, cx)
                }),
            )
        };
        bar = bar
            .child(
                div()
                    .id(eid("db-tbl-mode", key))
                    .flex()
                    .flex_row()
                    .flex_none()
                    .tooltip(|window, cx| {
                        gpui_component::tooltip::Tooltip::new(
                            "Inline: edit cells in the grid. Form: edit in the side form.",
                        )
                        .build(window, cx)
                    })
                    .child(segment("db-tbl-inline", "Inline", EditMode::Inline, cx))
                    .child(segment("db-tbl-form", "Form", EditMode::Form, cx)),
            )
            .child(choice_pill(
                eid("db-tbl-sql", key),
                "SQL preview",
                tab.show_sql,
                click(cx, key, |this, key, _, cx| this.db_table_toggle_sql(key, cx)),
            ))
            .child(muted(match view.pending {
                0 => "no pending changes".to_string(),
                1 => "1 row pending".to_string(),
                n => format!("{n} rows pending"),
            }))
            .when(tab.applying, |this| this.child(muted("Applying…")))
            .when(no_key, |this| {
                this.child(
                    div()
                        .flex_none()
                        .px_2()
                        .border(px(theme::hairline()))
                        .border_color(theme::warning())
                        .text_color(theme::warning())
                        .child("no primary key — edits match on all columns"),
                )
            });
    } else {
        bar = bar.child(choice_pill(
            eid("db-tbl-inspector", key),
            "Row inspector",
            tab.show_form,
            click(cx, key, |this, key, _, cx| {
                this.db_table_toggle_inspector(key, cx)
            }),
        ));
    }
    bar
}

/// The side form of the selected row.
fn row_form(view: &View, cx: &mut Context<AppState>) -> Div {
    let tab = view.tab;
    let key = view.key;
    let row = tab.selected.unwrap_or_default();
    let grid = view.widgets.grid.read(cx);
    let mark = grid.row_mark(row, cx);
    let refused = grid.row_error(row, cx);
    let (state, state_colour) = match mark {
        _ if tab.read_only(view.conn_ro) => ("read-only", theme::db_read_only()),
        RowMark::Clean => ("unchanged", theme::text_faint()),
        RowMark::Edited => ("edited", theme::accent()),
        RowMark::Inserted => ("new row", theme::success()),
        RowMark::Deleted => ("marked for deletion", theme::danger()),
    };

    let fields = view
        .widgets
        .fields
        .iter()
        .enumerate()
        .filter_map(|(col, field)| Some((col, field, tab.columns.get(col)?)))
        .map(|(col, field, meta)| {
            let error = tab.field_errors.get(col).cloned().flatten();
            let mut tags = vec![meta.native_type.clone()];
            if meta.is_pk {
                tags.push("PK".into());
            }
            if !meta.nullable {
                tags.push("NOT NULL".into());
            }
            if meta.computed {
                tags.push("computed".into());
            }
            // A text column whose value is JSON edits in the JSON dialog.
            let routed = field.input.read(cx).is_json();
            if routed && !matches!(meta.data_type, ubiq_proto::db::DataType::Json) {
                tags.push("JSON".into());
            }
            let forceable = json::json::can_force(&meta.data_type);
            let key = key.to_string();
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .text_size(theme::font(Family::Content, Role::Dense))
                        .child(
                            div()
                                .text_color(if error.is_some() {
                                    theme::danger()
                                } else {
                                    theme::text()
                                })
                                .child(SharedString::from(meta.name.clone())),
                        )
                        .child(
                            div()
                                .min_w(px(0.))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_color(theme::text_faint())
                                .child(SharedString::from(tags.join(" · "))),
                        )
                        .when(forceable, |this| {
                            this.child(div().flex_1()).child(
                                open_button(("db-tbl-json", col)).on_click(cx.listener(
                                    move |this, _: &ClickEvent, window, cx| {
                                        this.db_table_open_json_selected(&key, col, window, cx)
                                    },
                                )),
                            )
                        }),
                )
                .child(field.input.clone())
                .when_some(error, |this, message| {
                    this.child(
                        div()
                            .text_size(theme::font(Family::Content, Role::Dense))
                            .text_color(theme::danger())
                            .child(SharedString::from(message)),
                    )
                })
        })
        .collect::<Vec<_>>();

    div()
        .flex()
        .flex_col()
        .flex_none()
        .w(px(theme::scaled(FORM_WIDTH)))
        .h_full()
        .min_h(px(0.))
        .bg(theme::surface())
        .border_l(px(theme::hairline()))
        .border_color(theme::border())
        .child(
            div()
                .flex()
                .flex_row()
                .flex_none()
                .justify_between()
                .px_2()
                .h(px(theme::scaled(BAR_HEIGHT)))
                .items_center()
                .when(tab.read_only(view.conn_ro), |this| {
                    this.bg(theme::db_read_only_soft())
                })
                .border_b(px(theme::hairline()))
                .border_color(theme::border())
                .text_size(theme::font(Family::Chrome, Role::Label))
                .child(muted(format!("Row {}", row + 1)))
                .child(div().text_color(state_colour).child(state)),
        )
        .when_some(refused, |this, message| {
            this.child(strip("db-tbl-row-error", theme::danger(), message))
        })
        .child(
            div()
                .id(eid("db-tbl-form", key))
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .gap_2()
                .p_2()
                .children(fields),
        )
}

/// The SQL the pending edits would run: each as a statement, or the reason it has none.
fn preview(view: &View, cx: &mut Context<AppState>) -> impl IntoElement {
    let edits = view.widgets.grid.read(cx).edits(cx);
    let lines: Vec<(String, bool)> = if edits.is_empty() {
        vec![("-- no pending changes".to_string(), false)]
    } else {
        edits
            .iter()
            .map(|edit| match render_edit(view.kind, &view.tab.table, edit) {
                Ok(sql) => (format!("{sql};"), false),
                Err(error) => (format!("-- {error}"), true),
            })
            .collect()
    };
    div()
        .id(eid("db-tbl-preview", view.key))
        .flex()
        .flex_col()
        .flex_none()
        .h(px(theme::scaled(PREVIEW_HEIGHT)))
        .overflow_y_scroll()
        .px_2()
        .py_1()
        .bg(theme::surface_raised())
        .border_t(px(theme::hairline()))
        .border_color(theme::border())
        .font_family(theme::MONO_FONT)
        .text_size(theme::font(Family::Content, Role::Body))
        .child(section_label("SQL preview"))
        .children(lines.into_iter().map(|(text, error)| {
            div()
                .text_color(if error {
                    theme::danger()
                } else {
                    theme::text()
                })
                .child(SharedString::from(text))
        }))
}
