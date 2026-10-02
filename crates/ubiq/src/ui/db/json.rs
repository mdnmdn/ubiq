//! The JSON dialog: a large editor over one cell's value, drawn as an overlay by its table tab.
//!
//! **When it opens** is decided by [`json::route`]: a `json` / `jsonb` column always, and a
//! text-like column whenever its current value is a JSON object or array. Any text-like column can
//! also be forced to it with the `{ }` button ([`open_button`]). Opened from the grid (double-click
//! / Enter on the cell, or the `{ }` button of the inline editor) and from the side form, always
//! through the table tab, which owns the dialog and paints it over itself.
//!
//! The editor is `gpui-component`'s code editor in `"json"` mode. Every change re-validates with
//! `serde_json`; a syntax error is underlined through the editor's diagnostics and named with its
//! `line:col`. **Save** hands the text to `parse_input` exactly as typed (Format and Minify are
//! explicit buttons): a `json` column cannot be saved while invalid, a text column can ("not valid
//! JSON — saved as text"). **Set NULL** is offered when the column is nullable. A read-only dialog
//! (read-only tab, computed column, deleted row) can view, copy and Format, never Save.
//!
//! Formatting is done on the text, not through `serde_json::Value`, so key order and number
//! spelling survive it. [`json`] is the pure half (no widget, unit-tested).

use std::sync::Once;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, ClickEvent, ClipboardItem, Context, ElementId, Entity, EventEmitter,
    InteractiveElement as _, IntoElement, KeyBinding, MouseButton, ParentElement as _, Render,
    Rgba, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Subscription,
    Window, actions, div, px, relative,
};
use gpui_component::highlighter::{Diagnostic, DiagnosticSeverity};
use gpui_component::input::{Editor, EditorState, InputEvent, RopeExt as _};
use ubiq_db::value::parse_input;
use ubiq_proto::db::{ColumnMeta, DataType, Value};

use super::{button, read_only_badge};
use crate::theme::{self, Family, Role};

/// Everything about JSON text that needs no window.
pub mod json {
    use serde::de::IgnoredAny;
    use ubiq_proto::db::DataType;

    /// Where and why a text is not JSON.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct JsonError {
        /// What the parser says, without its own "at line L column C" tail.
        pub message: String,
        /// 1-based.
        pub line: usize,
        /// 1-based, in bytes.
        pub column: usize,
        /// Byte offset into the text, on a char boundary, at most `text.len()`.
        pub offset: usize,
    }

    /// The top-level shape of a valid document.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Kind {
        Object,
        Array,
        String,
        Number,
        Bool,
        Null,
    }

    impl Kind {
        pub fn label(self) -> &'static str {
            match self {
                Kind::Object => "object",
                Kind::Array => "array",
                Kind::String => "string",
                Kind::Number => "number",
                Kind::Bool => "boolean",
                Kind::Null => "null",
            }
        }
    }

    fn is_ws(c: char) -> bool {
        matches!(c, ' ' | '\t' | '\n' | '\r')
    }

    /// Validate `text` as one JSON document (any value, as `json` / `jsonb` accept).
    pub fn check(text: &str) -> Result<Kind, JsonError> {
        serde_json::from_str::<IgnoredAny>(text).map_err(|e| {
            let full = e.to_string();
            let message = match full.rsplit_once(" at line ") {
                Some((m, _)) => m.to_string(),
                None => full,
            };
            let (line, column) = (e.line().max(1), e.column().max(1));
            JsonError {
                message,
                line,
                column,
                offset: offset_of(text, line, column),
            }
        })?;
        Ok(match text.trim_start().chars().next() {
            Some('{') => Kind::Object,
            Some('[') => Kind::Array,
            Some('"') => Kind::String,
            Some('t' | 'f') => Kind::Bool,
            Some('n') => Kind::Null,
            _ => Kind::Number,
        })
    }

    /// The byte offset of 1-based `line` / `column`, clamped into the text and onto a char
    /// boundary.
    pub fn offset_of(text: &str, line: usize, column: usize) -> usize {
        let mut start = 0;
        for _ in 1..line {
            match text[start..].find('\n') {
                Some(i) => start += i + 1,
                None => {
                    start = text.len();
                    break;
                }
            }
        }
        let mut offset = (start + column.saturating_sub(1)).min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    /// A JSON object or array (not a bare scalar), after trimming: what makes a text column's
    /// value open the JSON dialog by itself.
    pub fn looks_like_json(text: &str) -> bool {
        let t = text.trim();
        (t.starts_with('{') || t.starts_with('[')) && check(t).is_ok()
    }

    /// Whether editing a cell of this type with this value opens the JSON dialog on its own.
    pub fn route(data_type: &DataType, value: Option<&str>) -> bool {
        match data_type {
            DataType::Json => true,
            dt if dt.is_text_like() => value.is_some_and(looks_like_json),
            _ => false,
        }
    }

    /// Whether the `{ }` button is offered: JSON columns and every text-like column.
    pub fn can_force(data_type: &DataType) -> bool {
        matches!(data_type, DataType::Json) || data_type.is_text_like()
    }

    /// Pretty-print with a two-space indent. Works on the text (strings, key order and number
    /// spelling are untouched); an invalid text is an error.
    pub fn format(text: &str) -> Result<String, JsonError> {
        check(text)?;
        Ok(pretty(text))
    }

    /// Drop every blank outside strings. An invalid text is an error.
    pub fn minify(text: &str) -> Result<String, JsonError> {
        check(text)?;
        Ok(compact(text))
    }

    /// `text` without the whitespace outside strings. Does not validate.
    pub fn compact(text: &str) -> String {
        preview(text, usize::MAX)
    }

    /// The first `max` characters of [`compact`]`(text)` on one line (control characters inside
    /// strings become `⏎`), with `…` when there was more. Stops reading at the limit, so a huge
    /// document costs no more than a small one.
    pub fn preview(text: &str, max: usize) -> String {
        let mut out = String::new();
        let mut count = 0;
        let (mut in_str, mut esc) = (false, false);
        for c in text.chars() {
            if !in_str && is_ws(c) {
                continue;
            }
            if count == max {
                out.push('…');
                return out;
            }
            count += 1;
            if in_str {
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    in_str = false;
                }
            } else if c == '"' {
                in_str = true;
            }
            out.push(if c.is_control() { '⏎' } else { c });
        }
        out
    }

    fn newline(out: &mut String, depth: usize) {
        out.push('\n');
        for _ in 0..depth {
            out.push_str("  ");
        }
    }

    /// Re-indent a valid document. Empty `{}` / `[]` stay on one line.
    fn pretty(text: &str) -> String {
        let mut out = String::with_capacity(text.len() * 2);
        let mut depth = 0usize;
        let (mut in_str, mut esc) = (false, false);
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if in_str {
                out.push(c);
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    in_str = false;
                }
                continue;
            }
            match c {
                '"' => {
                    in_str = true;
                    out.push(c);
                }
                '{' | '[' => {
                    out.push(c);
                    let close = if c == '{' { '}' } else { ']' };
                    if chars.clone().find(|n| !is_ws(*n)) == Some(close) {
                        for n in chars.by_ref() {
                            if n == close {
                                break;
                            }
                        }
                        out.push(close);
                    } else {
                        depth += 1;
                        newline(&mut out, depth);
                    }
                }
                '}' | ']' => {
                    depth = depth.saturating_sub(1);
                    newline(&mut out, depth);
                    out.push(c);
                }
                ',' => {
                    out.push(',');
                    newline(&mut out, depth);
                }
                ':' => out.push_str(": "),
                c if is_ws(c) => {}
                c => out.push(c),
            }
        }
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn looks_like_json_wants_an_object_or_array() {
            assert!(looks_like_json(r#"{"a": 1}"#));
            assert!(looks_like_json("  [1, 2]  "));
            assert!(looks_like_json("{}"));
            assert!(!looks_like_json("42"));
            assert!(!looks_like_json("\"x\""));
            assert!(!looks_like_json("null"));
            assert!(!looks_like_json("{not json}"));
            assert!(!looks_like_json("[1, 2"));
            assert!(!looks_like_json(""));
            assert!(!looks_like_json("hello {\"a\":1}"));
        }

        #[test]
        fn route_by_type_and_content() {
            let text = DataType::Text { max_len: None };
            assert!(route(&DataType::Json, None));
            assert!(route(&DataType::Json, Some("not even json")));
            assert!(route(&text, Some(r#"{"a":1}"#)));
            assert!(!route(&text, Some("plain")));
            assert!(!route(&text, None));
            assert!(!route(&DataType::Bool, Some("[1]")));
            assert!(route(&DataType::Other("xml".into()), Some("[1]")));
            assert!(can_force(&DataType::Json));
            assert!(can_force(&text));
            assert!(!can_force(&DataType::Date));
        }

        #[test]
        fn check_names_the_kind() {
            assert_eq!(check("{}"), Ok(Kind::Object));
            assert_eq!(check(" [1] "), Ok(Kind::Array));
            assert_eq!(check("\"x\""), Ok(Kind::String));
            assert_eq!(check("-1.5e3"), Ok(Kind::Number));
            assert_eq!(check("true"), Ok(Kind::Bool));
            assert_eq!(check("null"), Ok(Kind::Null));
        }

        #[test]
        fn error_position_is_line_column_and_offset() {
            let text = "{\n  \"a\": 1,\n  \"b\": ,\n}";
            let e = check(text).unwrap_err();
            assert_eq!((e.line, e.column), (3, 8));
            assert_eq!(&text[e.offset..e.offset + 1], ",");
            assert!(!e.message.contains(" at line "), "{}", e.message);
            assert!(!e.message.is_empty());
        }

        #[test]
        fn error_at_the_end_is_clamped() {
            let text = "{\"a\": 1";
            let e = check(text).unwrap_err();
            // serde_json reports the last character read; never past the text.
            assert!(e.offset >= text.len() - 1 && e.offset <= text.len());
            let e = check("").unwrap_err();
            assert_eq!(e.offset, 0);
        }

        #[test]
        fn offset_lands_on_a_char_boundary() {
            let text = "{\"é\": }";
            let e = check(text).unwrap_err();
            assert!(text.is_char_boundary(e.offset));
            assert_eq!(offset_of("ab\ncd", 2, 2), 4);
            assert_eq!(offset_of("ab", 9, 9), 2);
            assert_eq!(offset_of("é", 1, 2), 0);
        }

        #[test]
        fn format_indents_by_two_and_keeps_everything_else() {
            let text = r#"{"b":[1,2,{"c":null}],"a":1.50,"s":"x, {y}: \"z\"","e":{},"f":[ ]}"#;
            let got = format(text).unwrap();
            assert_eq!(
                got,
                "{\n  \"b\": [\n    1,\n    2,\n    {\n      \"c\": null\n    }\n  ],\n  \"a\": 1.50,\n  \"s\": \"x, {y}: \\\"z\\\"\",\n  \"e\": {},\n  \"f\": []\n}"
            );
            // Key order and number spelling survive; the result minifies back to the input.
            assert_eq!(minify(&got).unwrap(), text.replace("[ ]", "[]"));
        }

        #[test]
        fn format_and_minify_refuse_invalid_text() {
            assert!(format("{").is_err());
            assert!(minify("[1,]").is_err());
            assert_eq!(format("7").unwrap(), "7");
        }

        #[test]
        fn minify_keeps_blanks_inside_strings() {
            assert_eq!(
                minify("{ \"a b\" : \"c  d\" ,\n \"e\" : [ 1 , 2 ] }").unwrap(),
                r#"{"a b":"c  d","e":[1,2]}"#
            );
        }

        #[test]
        fn preview_is_one_line_and_truncated() {
            assert_eq!(
                preview("{ \"a\" : 1,\n \"b\": 2 }", 100),
                r#"{"a":1,"b":2}"#
            );
            assert_eq!(preview(r#"{"a":"xxxxxxxxxx"}"#, 6), r#"{"a":"…"#);
            assert_eq!(preview("[1,2]", 5), "[1,2]");
            assert_eq!(preview("\"a\nb\"", 10), "\"a⏎b\"");
            assert_eq!(compact(" [ 1 , 2 ] "), "[1,2]");
        }
    }
}

actions!(ubiq_db_json, [SaveJson, CancelJson]);

/// The key context the dialog's root carries, and the one the bindings are scoped to.
const CONTEXT: &str = "DbJson";

/// cmd-s / cmd-enter save and Esc cancels, once per process. Scoped to an `Input` *inside* the
/// dialog so they outrank the editor's own `secondary-enter` and `escape`, which bind at the
/// deepest node.
fn bind_keys(cx: &mut App) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        cx.bind_keys([
            KeyBinding::new("secondary-s", SaveJson, Some("DbJson > Input")),
            KeyBinding::new("secondary-enter", SaveJson, Some("DbJson > Input")),
            KeyBinding::new("escape", CancelJson, Some("DbJson > Input")),
        ]);
    });
}

/// What a dialog is opened on.
pub struct JsonSpec {
    /// `table.column`.
    pub title: String,
    pub meta: ColumnMeta,
    /// The cell's text; `None` = NULL (opens an empty editor).
    pub value: Option<String>,
    /// View, copy and Format only.
    pub read_only: bool,
}

/// What a [`JsonEditor`] tells its owner.
#[derive(Debug, Clone)]
pub enum JsonEvent {
    /// Save or Set NULL: the cell's new value, already through `parse_input`.
    Saved(Value),
    /// Closed without saving.
    Cancelled,
}

pub struct JsonEditor {
    spec: JsonSpec,
    editor: Entity<EditorState>,
    /// The text the dialog opened with, to know whether anything changed.
    initial: String,
    /// The editor's text as JSON.
    check: Result<json::Kind, json::JsonError>,
    /// `parse_input` refused the text on Save.
    error: Option<SharedString>,
    /// Esc on a changed text asks before discarding.
    confirm: bool,
    _subscription: Subscription,
}

impl EventEmitter<JsonEvent> for JsonEditor {}

impl JsonEditor {
    pub fn new(spec: JsonSpec, window: &mut Window, cx: &mut Context<Self>) -> Self {
        bind_keys(cx);
        let initial = spec.value.clone().unwrap_or_default();
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("json")
                .line_number(true)
                .soft_wrap(true)
        });
        editor.update(cx, |editor, cx| {
            editor.set_value(initial.clone(), window, cx);
            editor.set_readonly(spec.read_only, cx);
            editor.focus(window, cx);
        });
        let subscription = cx.subscribe(&editor, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.recheck(cx);
            }
        });
        let mut this = Self {
            spec,
            editor,
            initial,
            check: Err(json::check("").unwrap_err()),
            error: None,
            confirm: false,
            _subscription: subscription,
        };
        this.recheck(cx);
        this
    }

    fn text(&self, cx: &App) -> String {
        self.editor.read(cx).value().to_string()
    }

    fn dirty(&self, cx: &App) -> bool {
        self.text(cx) != self.initial
    }

    fn is_json_column(&self) -> bool {
        matches!(self.spec.meta.data_type, DataType::Json)
    }

    /// A json column cannot be saved while invalid; a text column always can.
    fn can_save(&self) -> bool {
        !self.spec.read_only && (!self.is_json_column() || self.check.is_ok())
    }

    /// Validate the text again and underline the error.
    fn recheck(&mut self, cx: &mut Context<Self>) {
        let text = self.text(cx);
        self.check = json::check(&text);
        self.error = None;
        self.confirm = false;
        // A json column's problem is an error; a text column's only a warning.
        let severity = if self.is_json_column() {
            DiagnosticSeverity::Error
        } else {
            DiagnosticSeverity::Warning
        };
        let problem = match &self.check {
            Err(e) if !text.trim().is_empty() => Some(e.clone()),
            _ => None,
        };
        self.editor.update(cx, |editor, cx| {
            let rope = editor.text().clone();
            let Some(set) = editor.diagnostics_mut() else {
                return;
            };
            set.reset(&rope);
            if let Some(error) = problem {
                // One character at the error; at the very end of the text, the last one.
                let mut start = error.offset.min(text.len());
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
                    .with_severity(severity),
                );
            }
            cx.notify();
        });
        cx.notify();
    }

    fn set_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.set_value(text, window, cx));
        self.editor
            .update(cx, |editor, cx| editor.focus(window, cx));
        self.recheck(cx);
    }

    fn format(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Ok(text) = json::format(&self.text(cx)) {
            self.set_text(text, window, cx);
        }
    }

    fn minify(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Ok(text) = json::minify(&self.text(cx)) {
            self.set_text(text, window, cx);
        }
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.text(cx)));
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if !self.can_save() {
            return;
        }
        match parse_input(Some(&self.text(cx)), &self.spec.meta) {
            Ok(value) => cx.emit(JsonEvent::Saved(value)),
            Err(e) => {
                self.error = Some(e.message.into());
                cx.notify();
            }
        }
    }

    fn set_null(&mut self, cx: &mut Context<Self>) {
        if self.spec.read_only || !self.spec.meta.nullable {
            return;
        }
        match parse_input(None, &self.spec.meta) {
            Ok(value) => cx.emit(JsonEvent::Saved(value)),
            Err(e) => {
                self.error = Some(e.message.into());
                cx.notify();
            }
        }
    }

    /// Esc / Cancel: close, asking first when a text was changed.
    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.confirm {
            // A second Esc on the question keeps the dialog.
            self.confirm = false;
            cx.notify();
        } else if !self.spec.read_only && self.dirty(cx) {
            self.confirm = true;
            cx.notify();
        } else {
            cx.emit(JsonEvent::Cancelled);
        }
    }

    /// The status line: a colour and a sentence.
    fn status(&self, cx: &App) -> (Rgba, String) {
        let text = self.text(cx);
        let empty = text.trim().is_empty();
        let json_column = self.is_json_column();
        match &self.check {
            Ok(kind) => (theme::success(), format!("Valid JSON · {}", kind.label())),
            Err(_) if empty && !json_column => (
                theme::text_faint(),
                if self.spec.value.is_none() {
                    "NULL".into()
                } else {
                    "empty — saved as ''".into()
                },
            ),
            Err(e) => {
                let place = format!("{}:{} {}", e.line, e.column, e.message);
                if json_column {
                    (theme::danger(), format!("Invalid JSON · {place}"))
                } else {
                    (
                        theme::warning(),
                        format!("not valid JSON — saved as text · {place}"),
                    )
                }
            }
        }
    }
}

/// The `{ }` button that opens (or forces) the JSON dialog.
pub fn open_button(id: impl Into<ElementId>) -> Stateful<gpui::Div> {
    div()
        .id(id.into())
        .flex_none()
        .px_1p5()
        .font_family(theme::MONO_FONT)
        .text_size(theme::font(Family::Content, Role::Meta))
        .text_color(theme::accent())
        .border(px(theme::hairline()))
        .border_color(theme::border())
        .cursor_pointer()
        .hover(|style| style.bg(theme::accent_soft()))
        // The click must not land on the cell or field under the button.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child("{ }")
}

impl Render for JsonEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let read_only = self.spec.read_only;
        let (status_colour, status) = self.status(cx);
        let valid = self.check.is_ok();
        let can_save = self.can_save();
        let confirm = self.confirm;
        let error = self.error.clone();
        let nullable = self.spec.meta.nullable && !read_only;

        let header = div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap_2()
            .child(
                div()
                    .font_family(theme::MONO_FONT)
                    .text_size(theme::font(Family::Content, Role::Title))
                    .child(SharedString::from(self.spec.title.clone())),
            )
            .child(
                div()
                    .text_size(theme::font(Family::Content, Role::Dense))
                    .text_color(theme::text_faint())
                    .child(SharedString::from(self.spec.meta.native_type.clone())),
            )
            .when(read_only, |this| this.child(read_only_badge("read-only")));

        let toolbar =
            div()
                .flex()
                .flex_row()
                .flex_none()
                .items_center()
                .gap_2()
                .child(button("json-format", "Format", false, valid).on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.format(window, cx)),
                ))
                .child(button("json-minify", "Minify", false, valid).on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.minify(window, cx)),
                ))
                .child(
                    button("json-copy", "Copy", false, true)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.copy(cx))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(theme::font(Family::Content, Role::Dense))
                        .text_color(status_colour)
                        .child(SharedString::from(status)),
                );

        let footer = div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap_2()
            .when(nullable, |this| {
                this.child(
                    button("json-null", "Set NULL", false, true)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_null(cx))),
                )
            })
            .child(div().flex_1())
            .child(
                button(
                    "json-cancel",
                    if read_only { "Close" } else { "Cancel" },
                    false,
                    true,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx))),
            )
            .when(!read_only, |this| {
                this.child(
                    button("json-save", "Save", true, can_save)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.save(cx))),
                )
            });

        let card = div()
            .id("json-editor")
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &SaveJson, _, cx| this.save(cx)))
            .on_action(cx.listener(|this, _: &CancelJson, _, cx| this.cancel(cx)))
            .flex()
            .flex_col()
            .gap_2()
            .w(relative(0.82))
            .max_w(px(1100.))
            .h(relative(0.82))
            .p_4()
            .bg(theme::surface_raised())
            .border_l(px(theme::accent_edge()))
            .border_color(if read_only {
                theme::db_read_only()
            } else {
                theme::accent()
            })
            .text_color(theme::text())
            .text_size(theme::font(Family::Content, Role::Body))
            .child(header)
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_hidden()
                    .border(px(theme::hairline()))
                    .border_color(theme::border())
                    .child(
                        Editor::new(&self.editor)
                            .h(relative(1.))
                            .p_0()
                            .border_0()
                            .font_family(theme::MONO_FONT)
                            .text_size(theme::font(Family::Content, Role::Body)),
                    ),
            )
            .when_some(error, |this, message| {
                this.child(
                    div()
                        .flex_none()
                        .text_size(theme::font(Family::Content, Role::Dense))
                        .text_color(theme::danger())
                        .child(message),
                )
            })
            .when(confirm, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_none()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .bg(theme::db_row_edited())
                        .text_size(theme::font(Family::Content, Role::Dense))
                        .child(div().flex_1().child("Discard your changes?"))
                        .child(button("json-keep", "Keep editing", false, true).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.confirm = false;
                                cx.notify();
                            }),
                        ))
                        .child(button("json-discard", "Discard", false, true).on_click(
                            cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(JsonEvent::Cancelled)),
                        )),
                )
            })
            .child(footer);

        // The scrim swallows every pointer event under the dialog.
        div()
            .id("json-editor-backdrop")
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .bg(theme::scrim())
            .child(card)
    }
}
