//! A file's change against a version-control base, as rows.
//!
//! The host computed the hunks; this draws them. No diff library reaches the interface, which is
//! the same discipline that keeps a VT parser out of the host.
//!
//! It is the chat transcript's `EDIT` block at file scale — one styled row per line, in status
//! tokens at low alpha — with a side-by-side layout as its second mode. Which of the two is on
//! screen is the open file's `layout`, so the header's toggle is the one Markdown already uses.

use gpui::{
    AnyElement, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};
use ubiq_proto::files::{DiffHunk, DiffRow, DiffRowKind, FileDiff};

use crate::state::editor::ViewLayout;
use crate::theme;
use crate::ui::kit::mono;

/// The gutter is wide enough for a five-figure line number and no wider.
const GUTTER: f32 = 44.0;
const ROW_TEXT: f32 = 11.5;

/// Draw the whole comparison. `Split` is side by side; every other layout is unified.
pub fn render(diff: &FileDiff, layout: ViewLayout) -> AnyElement {
    if diff.binary {
        return super::note("Not text · nothing to compare", theme::text_faint());
    }
    if diff.hunks.is_empty() {
        return super::note("No change against the base", theme::text_faint());
    }

    let side_by_side = matches!(layout, ViewLayout::Split);
    let mut body = super::surface().id("diff-body").overflow_scroll();

    for hunk in &diff.hunks {
        body = body.child(header(hunk));
        body = body.child(if side_by_side {
            columns(hunk)
        } else {
            unified(hunk)
        });
    }

    if diff.truncated {
        body = body.child(super::note(
            "The host stopped here · this is part of the change",
            theme::warning(),
        ));
    }

    body.into_any_element()
}

/// The `@@` line, kept as numbers rather than as text nobody has to parse back out.
fn header(hunk: &DiffHunk) -> impl IntoElement {
    let text = format!(
        "@@ -{},{} +{},{} @@",
        hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
    );
    div()
        .flex()
        .flex_none()
        .px_2()
        .py_1()
        .bg(theme::surface())
        .border_t_1()
        .border_color(theme::border())
        .child(mono(text, theme::text_faint()).text_size(px(ROW_TEXT)))
}

/// The colours a row's kind takes, from the status group at low alpha.
fn tones(kind: DiffRowKind) -> (gpui::Rgba, gpui::Rgba) {
    match kind {
        DiffRowKind::Added => (theme::success(), theme::success_soft()),
        DiffRowKind::Removed => (theme::danger(), theme::danger_soft()),
        DiffRowKind::Context => (theme::text_muted(), theme::pane_bg()),
    }
}

/// What the gutter draws for a row's marker.
fn marker(kind: DiffRowKind) -> &'static str {
    match kind {
        DiffRowKind::Added => "+",
        DiffRowKind::Removed => "\u{2212}",
        DiffRowKind::Context => " ",
    }
}

/// A line number, or the blank a row has on the side it is not on.
fn number(line: Option<u32>, colour: gpui::Rgba) -> impl IntoElement {
    let text: SharedString = match line {
        Some(n) => n.to_string().into(),
        None => "".into(),
    };
    div()
        .flex()
        .flex_none()
        .w(px(GUTTER))
        .justify_end()
        .pr_2()
        .child(mono(text, colour).text_size(px(ROW_TEXT)))
}

/// One hunk, old and new interleaved — the layout a terminal diff has.
fn unified(hunk: &DiffHunk) -> AnyElement {
    let rows = hunk.rows.iter().map(|row| {
        let (fg, bg) = tones(row.kind);
        div()
            .flex()
            .flex_none()
            .bg(bg)
            .child(number(row.old_line, theme::text_faint()))
            .child(number(row.new_line, theme::text_faint()))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .w(px(16.))
                    .justify_center()
                    .child(mono(marker(row.kind), fg).text_size(px(ROW_TEXT))),
            )
            .child(
                mono(row.text.clone(), fg)
                    .flex_1()
                    .min_w(px(0.))
                    .text_size(px(ROW_TEXT)),
            )
    });

    div()
        .flex()
        .flex_col()
        .flex_none()
        .children(rows.collect::<Vec<_>>())
        .into_any_element()
}

/// One hunk, old beside new.
///
/// A row that is on one side only leaves the other side blank rather than shifting it, so the two
/// columns stay line-for-line — which is the whole reason to draw it this way.
fn columns(hunk: &DiffHunk) -> AnyElement {
    let (old, new) = pair(&hunk.rows);

    div()
        .flex()
        .flex_none()
        .child(side(&old, true))
        .child(
            div()
                .flex()
                .flex_none()
                .w(px(1.))
                .bg(theme::border())
                .into_any_element(),
        )
        .child(side(&new, false))
        .into_any_element()
}

/// Walk a hunk into two equal-length columns.
///
/// A run of removals is zipped against the run of additions that follows it, so a changed line sits
/// opposite the line it replaced; whichever run is shorter is padded with blanks.
fn pair(rows: &[DiffRow]) -> (Vec<Option<&DiffRow>>, Vec<Option<&DiffRow>>) {
    let mut old: Vec<Option<&DiffRow>> = Vec::new();
    let mut new: Vec<Option<&DiffRow>> = Vec::new();
    let mut at = 0;

    while at < rows.len() {
        match rows[at].kind {
            DiffRowKind::Context => {
                old.push(Some(&rows[at]));
                new.push(Some(&rows[at]));
                at += 1;
            }
            _ => {
                let start = at;
                while at < rows.len() && rows[at].kind == DiffRowKind::Removed {
                    at += 1;
                }
                let removed = &rows[start..at];
                let added_from = at;
                while at < rows.len() && rows[at].kind == DiffRowKind::Added {
                    at += 1;
                }
                let added = &rows[added_from..at];

                // A run of neither is a row kind this build does not draw; step over it rather
                // than looping forever on it.
                if removed.is_empty() && added.is_empty() {
                    at += 1;
                    continue;
                }

                for slot in 0..removed.len().max(added.len()) {
                    old.push(removed.get(slot));
                    new.push(added.get(slot));
                }
            }
        }
    }

    (old, new)
}

/// One column of a side-by-side hunk.
fn side(rows: &[Option<&DiffRow>], is_old: bool) -> AnyElement {
    let drawn = rows.iter().map(|row| {
        let Some(row) = row else {
            // The blank opposite a line the other side does not have.
            return div()
                .flex()
                .flex_none()
                .h(px(ROW_TEXT + 6.))
                .bg(theme::pane_bg());
        };
        let (fg, bg) = tones(row.kind);
        let line = if is_old { row.old_line } else { row.new_line };
        div()
            .flex()
            .flex_none()
            .bg(bg)
            .child(number(line, theme::text_faint()))
            .child(
                mono(row.text.clone(), fg)
                    .flex_1()
                    .min_w(px(0.))
                    .px_1()
                    .text_size(px(ROW_TEXT)),
            )
    });

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .children(drawn.collect::<Vec<_>>())
        .into_any_element()
}
