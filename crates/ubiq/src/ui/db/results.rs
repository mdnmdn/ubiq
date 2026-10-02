//! The SQL tab's result grid: `gpui_component::table` over one statement's [`ResultSet`], read-only.
//!
//! The rows are held as the host sent them — `Vec<Vec<Value>>` beside their `ColumnMeta`s — and
//! every cell is drawn through [`cell_text`]: NULL as a faint italic `NULL`, an empty string as a
//! faint `''` (it is not NULL), a long value cut at [`MAX_CELL_CHARS`] with line breaks as `⏎`. The
//! value itself is never cut. Numbers align right. Text is the content family at the project's
//! zoom, in the mono face, the way the IDE editor draws.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::table::{Column, TableDelegate, TableState};
use gpui_component::tooltip::Tooltip;
use ubiq_proto::db::{ColumnMeta, ResultSet, Value};

use crate::theme::{self, Family, Role};

/// Longest cell text drawn; the value itself is kept whole.
const MAX_CELL_CHARS: usize = 300;
/// Rows sampled to size a column.
const WIDTH_SAMPLE_ROWS: usize = 50;
/// A mono glyph's width as a share of its size, near enough to size a column by.
const CHAR_EM: f32 = 0.62;
/// A column's narrowest and widest default, at `ui_scale = 1.0`.
const MIN_COL_WIDTH: f32 = 60.0;
const MAX_COL_WIDTH: f32 = 420.0;

/// The table's data source.
#[derive(Default)]
pub struct GridDelegate {
    columns: Vec<ColumnMeta>,
    rows: Vec<Vec<Value>>,
    widths: Vec<Pixels>,
}

impl GridDelegate {
    pub fn new(set: ResultSet) -> Self {
        let em = f32::from(theme::font(Family::Content, Role::Dense)) * CHAR_EM;
        let widths = set
            .columns
            .iter()
            .enumerate()
            .map(|(ix, meta)| column_width(meta, set.rows.iter().take(WIDTH_SAMPLE_ROWS), ix, em))
            .collect();
        Self {
            columns: set.columns,
            rows: set.rows,
            widths,
        }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
}

fn column_width<'a>(
    meta: &ColumnMeta,
    sample: impl Iterator<Item = &'a Vec<Value>>,
    ix: usize,
    em: f32,
) -> Pixels {
    let mut chars = meta.name.chars().count();
    for row in sample {
        if let Some(value) = row.get(ix) {
            chars = chars.max(cell_text(value).chars().count());
        }
    }
    let pad = theme::scaled(14.0);
    px((chars as f32 * em + pad).clamp(theme::scaled(MIN_COL_WIDTH), theme::scaled(MAX_COL_WIDTH)))
}

/// A cell's text on one line of at most [`MAX_CELL_CHARS`] characters.
pub fn cell_text(value: &Value) -> String {
    match value {
        Value::Null => return "NULL".to_string(),
        Value::Text(text) if text.is_empty() => return "''".to_string(),
        _ => {}
    }
    let text = value.to_string();
    let mut out: String = text
        .chars()
        .take(MAX_CELL_CHARS)
        .map(|c| {
            if matches!(c, '\n' | '\r' | '\t') {
                '⏎'
            } else {
                c
            }
        })
        .collect();
    if text.chars().nth(MAX_CELL_CHARS).is_some() {
        out.push('…');
    }
    out
}

impl TableDelegate for GridDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let meta = &self.columns[col_ix];
        let column = Column::new(col_ix.to_string(), meta.name.clone())
            .width(self.widths[col_ix])
            .min_width(px(theme::scaled(40.0)))
            .paddings(gpui::Edges {
                top: px(0.),
                bottom: px(0.),
                left: px(theme::scaled(6.0)),
                right: px(theme::scaled(6.0)),
            });
        if meta.data_type.is_numeric() {
            column.text_right()
        } else {
            column
        }
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let meta = &self.columns[col_ix];
        let tip = SharedString::from(if meta.native_type.is_empty() {
            meta.name.clone()
        } else {
            format!("{} · {}", meta.name, meta.native_type)
        });
        div()
            .id(("db-sql-th", col_ix))
            .size_full()
            .flex()
            .items_center()
            .text_size(theme::font(Family::Content, Role::Dense))
            .when(meta.data_type.is_numeric(), |this| this.justify_end())
            .child(SharedString::from(meta.name.clone()))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        div().id(("db-sql-row", row_ix))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        static NULL: Value = Value::Null;
        let value = self
            .rows
            .get(row_ix)
            .and_then(|row| row.get(col_ix))
            .unwrap_or(&NULL);
        let null = matches!(value, Value::Null);
        // `''` is not NULL: it is drawn as a faint pair of quotes rather than as nothing.
        let empty = matches!(value, Value::Text(text) if text.is_empty());
        let numeric = self.columns[col_ix].data_type.is_numeric();
        div()
            .size_full()
            .flex()
            .items_center()
            .font_family(theme::MONO_FONT)
            .text_size(theme::font(Family::Content, Role::Dense))
            .when(numeric, |this| this.justify_end())
            .when(null, |this| this.italic().text_color(theme::text_faint()))
            .when(empty, |this| this.text_color(theme::text_faint()))
            .when(!null && !empty, |this| this.text_color(theme::text()))
            .child(
                div()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(SharedString::from(cell_text(value))),
            )
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(theme::font(Family::Content, Role::Meta))
            .text_color(theme::text_faint())
            .child("No rows")
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        self.rows
            .get(row_ix)
            .and_then(|row| row.get(col_ix))
            .map(|value| value.to_string())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_and_empty_are_told_apart() {
        assert_eq!(cell_text(&Value::Null), "NULL");
        assert_eq!(cell_text(&Value::Text(String::new())), "''");
        assert_eq!(cell_text(&Value::Text("NULL".into())), "NULL");
        assert_eq!(cell_text(&Value::Int(7)), "7");
    }

    #[test]
    fn a_cell_is_one_line_and_bounded() {
        assert_eq!(cell_text(&Value::Text("a\nb\tc".into())), "a⏎b⏎c");
        let long = "x".repeat(MAX_CELL_CHARS + 10);
        let shown = cell_text(&Value::Text(long));
        assert_eq!(shown.chars().count(), MAX_CELL_CHARS + 1);
        assert!(shown.ends_with('…'));
    }
}
