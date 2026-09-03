//! The project search panel.
//!
//! A panel that searches across project files. The query is typed into an input field;
//! results arrive in batches grouped by file, with the total count and a truncation
//! indicator at the bottom.

use gpui::{AnyElement, Context, Focusable, IntoElement, ParentElement, Styled, Window, div, px};
use gpui_component::input::Input;

use crate::app::AppState;
use crate::theme;
use crate::ui::empty::empty_panel;
use crate::ui::kit::{filter_bar, mono, panel, panel_header};

pub fn render(app: &AppState, _window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let query_input = Input::new(&app.search.query).appearance(false);
    let focused = app
        .search
        .query
        .read(cx)
        .focus_handle(cx)
        .is_focused(_window);

    let results_body = if app.search.results.is_empty() {
        if app.search.active.is_some() {
            loading()
        } else {
            empty_panel("Type a query to search").into_any_element()
        }
    } else {
        results(app)
    };

    panel()
        .child(panel_header("Search", div()))
        .child(filter_bar(query_input, div(), focused))
        .child(results_body)
        .child(status_bar(app))
        .into_any_element()
}

fn results(app: &AppState) -> AnyElement {
    let mut rows: Vec<AnyElement> = Vec::new();

    for file in &app.search.results {
        rows.push(
            div()
                .h(px(22.))
                .px_3()
                .flex()
                .flex_none()
                .items_center()
                .child(
                    mono(file.rel_path.clone(), theme::accent())
                        .text_size(px(11.))
                        .overflow_hidden(),
                )
                .child(div().flex_1().min_w(px(0.)))
                .child(
                    mono(format!("{}", file.hits.len()), theme::text_faint())
                        .text_size(px(10.))
                        .flex_none(),
                )
                .into_any_element(),
        );

        for hit in &file.hits {
            let line = format!("{}", hit.line);
            rows.push(
                div()
                    .h(px(20.))
                    .pl_6()
                    .pr_3()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .child(
                        mono(line, theme::text_faint())
                            .text_size(px(10.))
                            .flex_none()
                            .w(px(36.)),
                    )
                    .child(
                        mono(hit.text.clone(), theme::text())
                            .text_size(px(11.))
                            .overflow_hidden(),
                    )
                    .into_any_element(),
            );
        }

        if file.truncated {
            rows.push(
                div()
                    .h(px(20.))
                    .pl_6()
                    .pr_3()
                    .flex()
                    .flex_none()
                    .items_center()
                    .child(mono("+ more hits truncated", theme::text_faint()).text_size(px(10.)))
                    .into_any_element(),
            );
        }
    }

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .overflow_hidden()
        .children(rows)
        .into_any_element()
}

fn loading() -> AnyElement {
    div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .items_center()
        .justify_center()
        .child(mono("Searching\u{2026}", theme::text_faint()))
        .into_any_element()
}

fn status_bar(app: &AppState) -> AnyElement {
    let status = if app.search.results.is_empty() {
        String::new()
    } else if app.search.finished {
        format!(
            "{} files \u{b7} {} hits",
            app.search.files_seen, app.search.total_hits
        )
    } else {
        format!(
            "{} files seen \u{b7} {} hits\u{2026}",
            app.search.files_seen, app.search.total_hits
        )
    };

    let truncation = if app.search.truncated {
        " \u{b7} truncated"
    } else {
        ""
    };

    div()
        .h(px(24.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .border_t_1()
        .border_color(theme::border())
        .child(mono(format!("{status}{truncation}"), theme::text_faint()).text_size(px(10.)))
        .into_any_element()
}
