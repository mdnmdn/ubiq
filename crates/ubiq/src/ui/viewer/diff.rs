//! A file's change against a version-control base, as rows.
//!
//! The host computed the hunks; this draws them. No diff library reaches the interface, which is
//! the same discipline that keeps a VT parser out of the host.
//!
//! It is the chat transcript's `EDIT` block at file scale — one styled row per line, in status
//! tokens at low alpha — with a side-by-side layout as its second mode. Which of the two is on
//! screen is the open file's `layout`, so the header's toggle is the one Markdown already uses.

use std::cell::RefCell;
use std::sync::Arc;

use gpui::{
    AnyElement, IntoElement, ListHorizontalSizingBehavior, ParentElement, SharedString, Styled,
    div, px, uniform_list,
};
use ubiq_proto::files::{DiffHunk, DiffRow, DiffRowKind, FileDiff};

use crate::state::editor::ViewLayout;
use crate::theme;
use crate::ui::kit::mono;

/// The gutter is wide enough for a five-figure line number and no wider.
const GUTTER: f32 = 44.0;
/// `side()`'s text carries `px_1` on both edges — the padding a pair column's content width has
/// to account for, on top of its share of the line.
const PAIR_TEXT_PAD: f32 = 8.0;
/// Every row in the list is this tall — a hunk header included, so the list is uniform and only
/// what is on screen is built. Sized from the text it holds rather than from a constant, the way
/// `kit::row_height` is: the content family's base is the project's, so a row that kept a fixed
/// height would clip its own text the moment the project zooms.
fn row_height() -> f32 {
    f32::from(theme::font(theme::Family::Content, theme::Role::Label)) + 6.0
}

/// One line, as the list holds it: the host's row with its text already a `SharedString`, so
/// drawing it is a refcount rather than a copy of the line.
#[derive(Clone)]
struct Line {
    kind: DiffRowKind,
    old_line: Option<u32>,
    new_line: Option<u32>,
    text: SharedString,
}

impl Line {
    fn of(row: &DiffRow) -> Self {
        Self {
            kind: row.kind,
            old_line: row.old_line,
            new_line: row.new_line,
            text: row.text.clone().into(),
        }
    }
}

/// One row of the flattened comparison. The hunks are one list rather than a chain of nested
/// containers, which is what lets `uniform_list` build the thirty rows on screen instead of all ten
/// thousand of them.
enum Flat {
    Header(SharedString),
    /// A unified row: old and new interleaved.
    Line(Line),
    /// A side-by-side row: whichever of the two sides has a line here.
    Pair(Option<Line>, Option<Line>),
}

/// A flattened comparison, plus the character count of its widest line.
///
/// The count travels with the rows rather than a pixel width: a row is drawn at whatever the
/// project's font resolves to *now*, and the cache below outlives a zoom change, so the width has
/// to be derived at draw time, not baked in here.
struct Rows {
    flat: Vec<Flat>,
    max_chars: usize,
}

/// The characters a row puts on screen, whichever kind it is — the property `Rows::max_chars`
/// is the largest of.
fn row_chars(row: &Flat) -> usize {
    match row {
        Flat::Header(_) => 0,
        Flat::Line(line) => line.text.chars().count(),
        Flat::Pair(old, new) => {
            let side = |line: &Option<Line>| line.as_ref().map_or(0, |l| l.text.chars().count());
            side(old).max(side(new))
        }
    }
}

/// Draw the whole comparison. `Split` is side by side; every other layout is unified.
pub fn render(diff: &FileDiff, layout: ViewLayout) -> AnyElement {
    if diff.binary {
        return super::note("Not text · nothing to compare", theme::text_faint());
    }
    if diff.hunks.is_empty() {
        return super::note("No change against the base", theme::text_faint());
    }

    let side_by_side = matches!(layout, ViewLayout::Split);
    let rows = flattened(diff, side_by_side);
    let count = rows.flat.len();
    let width = content_width(rows.max_chars, side_by_side);

    let mut body = super::surface().child(
        uniform_list("diff-body", count, move |range, _window, _cx| {
            range
                .filter_map(|index| rows.flat.get(index).map(|row| draw(row, width)))
                .collect::<Vec<AnyElement>>()
        })
        // A uniform row cannot grow, so the width has to be known up front rather than measured
        // from item 0 (almost always a short `@@` header) — see `content_width`.
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        .flex_1()
        .min_h(px(0.)),
    );

    if diff.truncated {
        body = body.child(div().flex().flex_none().child(super::note(
            "The host stopped here · this is part of the change",
            theme::warning(),
        )));
    }

    body.into_any_element()
}

/// The rows of one comparison, built once per diff rather than once per frame.
///
/// ponytail: memoised here, keyed on the diff's address and its shape, because the flattened rows
/// have nowhere better to live — the `FileDiff` sits in `FileBody::Diff` (`state/editor.rs`) and on
/// `GitView`, and neither was in scope for this pass. Upgrade: build this beside the `FileDiff`
/// when the reply lands and hand it in, and this cache goes away.
fn flattened(diff: &FileDiff, side_by_side: bool) -> Arc<Rows> {
    type Key = (usize, usize, usize, bool, bool);
    thread_local! {
        // Already `const`: allowed because the lint fires on this toolchain regardless.
        #[allow(clippy::missing_const_for_thread_local)]
        static CACHE: RefCell<Vec<(Key, Arc<Rows>)>> = const { RefCell::new(Vec::new()) };
    }

    let lines: usize = diff.hunks.iter().map(|hunk| hunk.rows.len()).sum();
    let key: Key = (
        diff as *const FileDiff as usize,
        diff.hunks.len(),
        lines,
        diff.truncated,
        side_by_side,
    );

    CACHE.with_borrow_mut(|cache| {
        if let Some((_, rows)) = cache.iter().find(|(held, _)| *held == key) {
            return rows.clone();
        }
        let rows = Arc::new(flatten(diff, side_by_side));
        // Two diffs can be on screen at once — a diff tab and the git screen's pane — and a
        // layout toggle keys a third. Anything older than that is a comparison nobody is reading.
        if cache.len() >= 4 {
            cache.remove(0);
        }
        cache.push((key, rows.clone()));
        rows
    })
}

/// Walk the hunks into one list of rows, header rows included.
fn flatten(diff: &FileDiff, side_by_side: bool) -> Rows {
    let mut rows = Vec::new();
    for hunk in &diff.hunks {
        rows.push(Flat::Header(header_text(hunk)));
        if side_by_side {
            let (old, new) = pair(&hunk.rows);
            rows.extend(
                old.into_iter()
                    .zip(new)
                    .map(|(old, new)| Flat::Pair(old.map(Line::of), new.map(Line::of))),
            );
        } else {
            rows.extend(hunk.rows.iter().map(|row| Flat::Line(Line::of(row))));
        }
    }
    let max_chars = rows.iter().map(row_chars).max().unwrap_or(0);
    Rows {
        flat: rows,
        max_chars,
    }
}

/// A monospace glyph's rendered width, assumed as a fraction of the font's resolved size.
///
/// ponytail: `render()` builds its tree eagerly with no `Window` in scope to shape a real glyph
/// and measure it — `vendor/gpui-terminal`'s renderer does exactly that
/// (`TerminalRenderer::measure_cell`, via `text_system().shape_line`) but needs one. 0.6 is the
/// same ratio that renderer assumes before its first real measurement. Upgrade: thread a `Window`
/// down to here and shape the widest line for real, if the approximation ever visibly clips.
fn char_advance() -> f32 {
    f32::from(theme::font(theme::Family::Content, theme::Role::Label)) * 0.6
}

/// The width to give the list — and every row in it — so a line longer than the pane is reached by
/// scrolling instead of clipped: the widest line's character count, in the font in effect now, plus
/// the chrome around it.
fn content_width(max_chars: usize, side_by_side: bool) -> f32 {
    let text = max_chars as f32 * char_advance();
    if side_by_side {
        // Two columns, each wide enough for a full line, with the divider between them.
        2.0 * (GUTTER + PAIR_TEXT_PAD + text) + 1.0
    } else {
        2.0 * GUTTER + 16.0 + text
    }
}

/// One row of the list, whichever kind it is, at the list's known content width.
fn draw(row: &Flat, content_width: f32) -> AnyElement {
    match row {
        Flat::Header(text) => header(text.clone(), content_width),
        Flat::Line(line) => unified(line, content_width),
        Flat::Pair(old, new) => columns(old.as_ref(), new.as_ref(), content_width),
    }
}

/// The `@@` line, kept as numbers rather than as text nobody has to parse back out.
fn header_text(hunk: &DiffHunk) -> SharedString {
    format!(
        "@@ -{},{} +{},{} @@",
        hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
    )
    .into()
}

fn header(text: SharedString, content_width: f32) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .w(px(content_width))
        .h(px(row_height()))
        .px_2()
        .items_center()
        .bg(theme::surface())
        .border_t_1()
        .border_color(theme::border())
        .child(
            mono(text, theme::text_faint())
                .text_size(theme::font(theme::Family::Content, theme::Role::Label)),
        )
        .into_any_element()
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
        .child(
            mono(text, colour).text_size(theme::font(theme::Family::Content, theme::Role::Label)),
        )
}

/// One row, old and new interleaved — the layout a terminal diff has.
fn unified(row: &Line, content_width: f32) -> AnyElement {
    let (fg, bg) = tones(row.kind);
    div()
        .flex()
        .flex_none()
        .w(px(content_width))
        .h(px(row_height()))
        .items_center()
        .bg(bg)
        .child(number(row.old_line, theme::text_faint()))
        .child(number(row.new_line, theme::text_faint()))
        .child(
            div().flex().flex_none().w(px(16.)).justify_center().child(
                mono(marker(row.kind), fg)
                    .text_size(theme::font(theme::Family::Content, theme::Role::Label)),
            ),
        )
        .child(
            mono(row.text.clone(), fg)
                .flex_1()
                .min_w(px(0.))
                .whitespace_nowrap()
                .overflow_hidden()
                .text_size(theme::font(theme::Family::Content, theme::Role::Label)),
        )
        .into_any_element()
}

/// One row, old beside new.
///
/// A row that is on one side only leaves the other side blank rather than shifting it, so the two
/// columns stay line-for-line — which is the whole reason to draw it this way.
fn columns(old: Option<&Line>, new: Option<&Line>, content_width: f32) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .w(px(content_width))
        .h(px(row_height()))
        .child(side(old, true))
        .child(div().flex().flex_none().w(px(1.)).bg(theme::border()))
        .child(side(new, false))
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

/// One side of a side-by-side row.
fn side(row: Option<&Line>, is_old: bool) -> AnyElement {
    let Some(row) = row else {
        // The blank opposite a line the other side does not have.
        return div()
            .flex()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .bg(theme::pane_bg())
            .into_any_element();
    };
    let (fg, bg) = tones(row.kind);
    let line = if is_old { row.old_line } else { row.new_line };
    div()
        .flex()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .items_center()
        .bg(bg)
        .child(number(line, theme::text_faint()))
        .child(
            mono(row.text.clone(), fg)
                .flex_1()
                .min_w(px(0.))
                .px_1()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_size(theme::font(theme::Family::Content, theme::Role::Label)),
        )
        .into_any_element()
}
