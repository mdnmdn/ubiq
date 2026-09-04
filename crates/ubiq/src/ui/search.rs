//! The project search panel.
//!
//! A panel that searches across project files. The query is typed into an input field;
//! results arrive in batches grouped by file, with the total count and a truncation
//! indicator at the bottom.

use std::ops::Range;

use gpui::{
    AnyElement, Context, Focusable, HighlightStyle, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, StyledText, Window, div, px,
};
use gpui_component::input::Input;
use ubiq_proto::search::SearchError;

use crate::app::AppState;
use crate::theme;
use crate::ui::empty::empty_panel;
use crate::ui::kit::{filter_bar, mono, panel, row_height};

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
        .child(filter_bar(query_input, options, focused))
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

/// How many hit rows a panel draws. Past it the list stops and the status line says so: a query
/// like `e` matches tens of thousands of lines, and drawing them all is a stall, not a result.
pub const SHOWN_HITS: usize = 500;

fn results(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let mut rows: Vec<AnyElement> = Vec::new();
    let mut drawn = 0usize;
    // The results read against the tree and the editor, so they follow the same project font size,
    // and a row is as tall as that size asks for.
    let font = app.ui_font_size_or_default(cx) - 0.5;
    let row = row_height(font);

    for file in &app.search.results {
        if drawn >= SHOWN_HITS {
            break;
        }
        let path = file.rel_path.clone();
        rows.push(
            div()
                .id(gpui::ElementId::Name(format!("search-file-{path}").into()))
                .cursor_pointer()
                .hover(|this| this.bg(theme::hover()))
                .on_click(cx.listener(move |this, _, _, cx| this.select_file(path.clone(), cx)))
                .h(px(row + 2.))
                .px_3()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .child(
                    mono(file.rel_path.clone(), theme::accent())
                        .text_size(px(font))
                        .flex_1()
                        .min_w(px(0.))
                        .truncate(),
                )
                .child(
                    mono(format!("{}", file.hits.len()), theme::text_faint())
                        .text_size(px(font - 1.))
                        .flex_none(),
                )
                .into_any_element(),
        );

        for hit in &file.hits {
            if drawn >= SHOWN_HITS {
                break;
            }
            drawn += 1;
            // The number and the line are one text element, not two boxes: a row is one line, and
            // the gutter lines up because the font is monospaced.
            let (body, marks) = hit_line(hit);
            let gutter = format!("{:>5}  ", hit.line);
            let shift = gutter.len();
            let text = format!("{gutter}{body}");
            let highlights = std::iter::once((
                0..shift,
                HighlightStyle {
                    color: Some(theme::text_faint().into()),
                    ..Default::default()
                },
            ))
            .chain(marks.into_iter().map(move |range| {
                (
                    range.start + shift..range.end + shift,
                    HighlightStyle {
                        color: Some(theme::text().into()),
                        background_color: Some(theme::accent_soft().into()),
                        ..Default::default()
                    },
                )
            }));
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
                    .h(px(row))
                    .px_3()
                    .flex()
                    .flex_none()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .font_family(theme::MONO_FONT)
                            .text_size(px(font))
                            // The line is the row: a shorter line box would draw a match's
                            // highlight offset from the glyphs it marks, and a taller one grows
                            // the row, since a flex item's content is its minimum size.
                            .line_height(px(row))
                            .text_color(theme::text_muted())
                            // `truncate` rather than `whitespace_nowrap`: the ellipsis is what
                            // makes the measure cut the line to the row's width. Nowrap alone
                            // still lets the text lay out over two lines and doubles the row.
                            .truncate()
                            .child(StyledText::new(text).with_highlights(highlights)),
                    )
                    .into_any_element(),
            );
        }

        if file.truncated {
            rows.push(
                div()
                    .h(px(row))
                    .px_3()
                    .flex()
                    .flex_none()
                    .items_center()
                    .child(
                        mono("+ more hits truncated", theme::text_faint()).text_size(px(font - 1.)),
                    )
                    .into_any_element(),
            );
        }
    }

    div()
        .id("search-results")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .children(rows)
        .into_any_element()
}

/// How much of a hit line sits ahead of its first match. Only the head is windowed: a long tail is
/// left for the row to clip, so a wide panel shows as much of the line as it has room for instead
/// of stopping at a fixed column.
const HEAD_ROOM: usize = 24;

/// The hit line as it is drawn — indent trimmed, head windowed — and the byte ranges inside that
/// text which matched.
fn hit_line(hit: &ubiq_proto::search::LineHit) -> (String, Vec<Range<usize>>) {
    let trimmed = hit.text.trim_start();
    let indent = hit.text.len() - trimmed.len();
    let rebase = |at: u32, cut: usize, pad: usize| {
        (at as usize)
            .saturating_sub(indent)
            .saturating_sub(cut)
            .saturating_add(pad)
            .min(trimmed.len() - cut + pad)
    };

    let first = hit
        .ranges
        .first()
        .map_or(0, |(start, _)| (*start as usize).saturating_sub(indent));
    if first <= HEAD_ROOM {
        let marks = hit
            .ranges
            .iter()
            .map(|(start, end)| rebase(*start, 0, 0)..rebase(*end, 0, 0))
            .collect();
        return (trimmed.to_string(), marks);
    }

    // The cut lands on a character boundary, HEAD_ROOM bytes of context before the match.
    let mut cut = first - HEAD_ROOM;
    while !trimmed.is_char_boundary(cut) {
        cut -= 1;
    }
    let lead = '\u{2026}';
    let pad = lead.len_utf8();
    let marks = hit
        .ranges
        .iter()
        .map(|(start, end)| rebase(*start, cut, pad)..rebase(*end, cut, pad))
        .collect();
    (format!("{lead}{}", &trimmed[cut..]), marks)
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

    // What the panel drew, when it drew less than the search found.
    let shown = if app.search.total_hits > SHOWN_HITS {
        format!(" \u{b7} shown first {SHOWN_HITS}")
    } else {
        String::new()
    };

    status_line(mono(
        format!("{status}{truncation}{shown}"),
        theme::text_faint(),
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::search::LineHit;

    fn hit(text: &str, at: u32) -> LineHit {
        LineHit {
            line: 1,
            text: text.to_string(),
            ranges: vec![(at, at + 4)],
        }
    }

    #[test]
    fn short_lines_only_lose_their_indent() {
        let (text, marks) = hit_line(&hit("    term here", 4));
        assert_eq!(text, "term here");
        assert_eq!(&text[marks[0].clone()], "term");
    }

    #[test]
    fn a_late_match_keeps_its_mark_and_its_tail() {
        let line = format!("{}term{}", "x".repeat(200), "y".repeat(200));
        let (text, marks) = hit_line(&hit(&line, 200));
        assert!(text.starts_with('\u{2026}'));
        assert_eq!(&text[marks[0].clone()], "term");
        // The tail is the row's to clip, not this function's to cut.
        assert!(text.ends_with("yyy"));
    }
}
