//! The project search panel.
//!
//! A panel that searches across project files. The query is typed into an input field;
//! results arrive in batches grouped by file, with the total count and a truncation
//! indicator at the bottom.

use gpui::{
    AnyElement, Context, Focusable, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::input::Input;
use ubiq_proto::search::SearchError;

use crate::app::AppState;
use crate::theme;
use crate::ui::empty::empty_panel;
use crate::ui::kit::{filter_bar, mono, panel};

pub fn render(app: &AppState, _window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let query_input = Input::new(&app.search.query).appearance(false);
    let focused = app
        .search
        .query
        .read(cx)
        .focus_handle(cx)
        .is_focused(_window);

    let options = div()
        .flex()
        .flex_none()
        .items_center()
        .gap_0p5()
        .child(glyph_toggle(
            "search-case",
            "Aa",
            app.search.case_sensitive,
            cx.listener(|this, _, _, cx| this.toggle_search_case(cx)),
        ))
        .child(glyph_toggle(
            "search-word",
            "W",
            app.search.whole_word,
            cx.listener(|this, _, _, cx| this.toggle_search_whole_word(cx)),
        ))
        .child(glyph_toggle(
            "search-regex",
            ".*",
            app.search.regex,
            cx.listener(|this, _, _, cx| this.toggle_search_regex(cx)),
        ));

    let results_body = if app.search.results.is_empty() {
        if app.search.active.is_some() {
            loading()
        } else {
            empty_panel("Type a query to search").into_any_element()
        }
    } else {
        results(app, cx)
    };

    panel()
        // No header: the dock's tab already says "Search" and the field already carries the icon.
        // The pad is what that header used to give the field between it and the tab strip.
        .child(
            div()
                .flex_none()
                .pt_1p5()
                .child(filter_bar(query_input, options, focused)),
        )
        .child(results_body)
        .child(status_bar(app))
        .into_any_element()
}

/// One of the query's three facets, drawn the way every editor draws them: the bare glyph, lit
/// when the facet is on and faint when it is off. No pill and no dot — three bordered chips inside
/// the bordered field they sit in is a box in a box in a box.
fn glyph_toggle(
    id: &'static str,
    label: &'static str,
    active: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let colour = if active {
        theme::accent()
    } else {
        theme::text_faint()
    };
    div()
        .id(id)
        .size(px(18.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(mono(label, colour).text_size(px(11.)))
        .on_click(on_click)
}

fn results(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let mut rows: Vec<AnyElement> = Vec::new();

    for file in &app.search.results {
        let path = file.rel_path.clone();
        rows.push(
            div()
                .id(gpui::ElementId::Name(format!("search-file-{path}").into()))
                .cursor_pointer()
                .hover(|this| this.bg(theme::hover()))
                .on_click(cx.listener(move |this, _, _, cx| this.select_file(path.clone(), cx)))
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
            // §9's destination is the file: there is no open-at-line yet, so a hit row opens the
            // file and the caret stays where the editor puts it.
            let path = file.rel_path.clone();
            rows.push(
                div()
                    .id(gpui::ElementId::Name(
                        format!("search-hit-{path}-{}", hit.line).into(),
                    ))
                    .cursor_pointer()
                    .hover(|this| this.bg(theme::hover()))
                    .on_click(cx.listener(move |this, _, _, cx| this.select_file(path.clone(), cx)))
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
    // What stopped the search takes the line; the results it had are still drawn above it.
    if let Some(error) = &app.search.error {
        let said = match error {
            SearchError::Root => "the project's folder has gone".to_string(),
            SearchError::BadQuery(why) => format!("bad query: {why}"),
            SearchError::Walk(why) => format!("the walk failed: {why}"),
            SearchError::BadFilter(why) => format!("bad filter: {why}"),
        };
        return status_line(mono(said, theme::danger()));
    }

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

    status_line(mono(format!("{status}{truncation}"), theme::text_faint()))
}

fn status_line(text: impl IntoElement) -> AnyElement {
    div()
        .h(px(24.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .border_t_1()
        .border_color(theme::border())
        .child(div().text_size(px(10.)).child(text))
        .into_any_element()
}
