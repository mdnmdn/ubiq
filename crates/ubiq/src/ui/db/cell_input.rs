//! One cell's input, for the row form and the grid's inline editor.
//!
//! [`CellInput`] wraps a `gpui-component` [`InputState`] and the one thing a plain input cannot
//! hold: whether the cell is **NULL**. The rule, in [`rule`] (no widget, unit-tested):
//!
//! - on a text-like column (`Text`, `Char`, `Other`) an empty field is the empty string `''`;
//! - **Backspace on an already empty field makes it NULL** — if the column is nullable, otherwise
//!   a hint says why not;
//! - **typing a space on a NULL field makes it `''`** (the space is consumed); any other character
//!   starts a normal value;
//! - on every other column, empty means NULL when nullable (and a validation error when not).
//!
//! NULL is drawn as an italic faint label over the empty field, so `''` (nothing) and NULL differ.
//! The widget's value is an `Option<String>` (`None` = NULL) that goes straight to `parse_input`.
//! Date and date-time columns get a calendar picker (a date-time is the date picker beside a text
//! field for `HH:MM[:SS]`, composed to `YYYY-MM-DD HH:MM:SS`, any offset kept); a time column is a
//! plain field that `parse_input` validates; a **boolean** column is a dropdown (`true` / `false`,
//! plus `NULL` when nullable). Form and inline editor share this one widget; `inline` only changes
//! the chrome and the keys (see [`CellInput::new`]).
//!
//! **Inline geometry.** The inline editor is an absolutely positioned box that covers the whole
//! cell — the table pads its cells, and the editor cancels that padding with negative insets and
//! puts the same padding back inside — so the text does not move when editing starts, the row
//! height does not change, and the focus / invalid outline is an inset overlay (no layout). The
//! pickers' popups are `deferred` by `gpui-component` itself, which paints them at the window
//! level with no clip, so they escape the table's `overflow_hidden` cells.

use chrono::NaiveDate;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Context, Entity, EventEmitter, Focusable as _, InteractiveElement as _,
    IntoElement, KeyDownEvent, Keystroke, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use gpui_component::calendar::Date;
use gpui_component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_component::input::{
    Backspace, Escape, IndentInline, Input, InputEvent, InputState, OutdentInline,
};
use gpui_component::select::{SearchableVec, Select, SelectEvent, SelectState};
use gpui_component::{Disableable as _, Sizable as _, Size};
use ubiq_proto::db::{ColumnMeta, DataType};

use super::json::{json, open_button};
use crate::theme::{self, Family, Role};

/// The boolean dropdown's choices, as pure functions (no widget, unit-tested).
pub mod bool_pick {
    pub const TRUE: &str = "true";
    pub const FALSE: &str = "false";
    pub const NULL: &str = "NULL";

    /// The dropdown's items.
    pub fn items(nullable: bool) -> Vec<&'static str> {
        let mut items = vec![TRUE, FALSE];
        if nullable {
            items.push(NULL);
        }
        items
    }

    /// The item that shows `value` (`None` = NULL); `None` = no item (a NOT NULL column with no
    /// value yet, or text that is no boolean).
    pub fn choice(value: Option<&str>, nullable: bool) -> Option<&'static str> {
        match value {
            None => nullable.then_some(NULL),
            Some(v) if v.eq_ignore_ascii_case(TRUE) => Some(TRUE),
            Some(v) if v.eq_ignore_ascii_case(FALSE) => Some(FALSE),
            Some(_) => None,
        }
    }

    /// The input for `parse_input` that a chosen item stands for.
    pub fn value(choice: Option<&str>) -> Option<String> {
        choice.filter(|c| *c != NULL).map(str::to_string)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn null_is_offered_only_when_nullable() {
            assert_eq!(items(true), ["true", "false", "NULL"]);
            assert_eq!(items(false), ["true", "false"]);
        }

        #[test]
        fn values_map_to_items_and_back() {
            assert_eq!(choice(Some("true"), false), Some(TRUE));
            assert_eq!(choice(Some("FALSE"), true), Some(FALSE));
            assert_eq!(choice(None, true), Some(NULL));
            assert_eq!(choice(None, false), None);
            assert_eq!(choice(Some("maybe"), true), None);
            assert_eq!(value(Some(TRUE)), Some("true".into()));
            assert_eq!(value(Some(NULL)), None);
            assert_eq!(value(None), None);
        }
    }
}

/// The NULL / empty-string rule as pure functions of the field's state.
pub mod rule {
    /// What the rule needs to know about a column.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Shape {
        /// `Text` / `Char` / `Other`: empty is `''`, whitespace is data.
        pub text_like: bool,
        pub nullable: bool,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Key {
        Backspace,
        Space,
    }

    /// What a key does to the field.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Effect {
        /// Not the rule's business: the input handles the key.
        Pass,
        /// The field becomes NULL (the key is consumed).
        SetNull,
        /// The field becomes `''` (the key is consumed).
        SetEmpty,
        /// The key is consumed and nothing changes: the column is NOT NULL.
        Refuse,
    }

    /// A key arrived at a field that is `null` (NULL) or not, with `empty` text.
    pub fn on_key(shape: Shape, null: bool, empty: bool, key: Key) -> Effect {
        if !shape.text_like {
            return Effect::Pass;
        }
        match key {
            Key::Backspace if empty && !null => {
                if shape.nullable {
                    Effect::SetNull
                } else {
                    Effect::Refuse
                }
            }
            Key::Space if null => Effect::SetEmpty,
            _ => Effect::Pass,
        }
    }

    /// The NULL flag after the input's text changed to `text`: on a text-like column typing
    /// anything leaves NULL, an emptied field keeps whatever it was (a deletion gives `''`).
    /// Other columns have no flag — their NULL is "empty" ([`value`]).
    pub fn after_change(shape: Shape, null: bool, text: &str) -> bool {
        shape.text_like && null && text.is_empty()
    }

    /// The field's input for `parse_input`: `None` = NULL.
    pub fn value(shape: Shape, null: bool, text: &str) -> Option<String> {
        if shape.text_like {
            (!null).then(|| text.to_string())
        } else if shape.nullable && text.trim().is_empty() {
            None
        } else {
            Some(text.to_string())
        }
    }

    /// Whether the placeholder (NULL / default / generated) is drawn: an empty field that is not
    /// the deliberate `''` of a text column.
    pub fn show_placeholder(shape: Shape, null: bool, text: &str) -> bool {
        text.is_empty() && !(shape.text_like && !null)
    }

    /// The flag and the field text that show `value` (`None` = NULL).
    pub fn load(shape: Shape, value: Option<&str>) -> (bool, String) {
        (
            shape.text_like && value.is_none(),
            value.unwrap_or_default().to_string(),
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const TEXT: Shape = Shape {
            text_like: true,
            nullable: true,
        };
        const TEXT_NOT_NULL: Shape = Shape {
            text_like: true,
            nullable: false,
        };
        const NUM: Shape = Shape {
            text_like: false,
            nullable: true,
        };

        #[test]
        fn backspace_on_empty_text_makes_null_if_allowed() {
            assert_eq!(on_key(TEXT, false, true, Key::Backspace), Effect::SetNull);
            assert_eq!(
                on_key(TEXT_NOT_NULL, false, true, Key::Backspace),
                Effect::Refuse
            );
            // Not empty, or already NULL: the input deletes (or has nothing to delete).
            assert_eq!(on_key(TEXT, false, false, Key::Backspace), Effect::Pass);
            assert_eq!(on_key(TEXT, true, true, Key::Backspace), Effect::Pass);
        }

        #[test]
        fn space_on_null_makes_the_empty_string() {
            assert_eq!(on_key(TEXT, true, true, Key::Space), Effect::SetEmpty);
            assert_eq!(
                on_key(TEXT_NOT_NULL, true, true, Key::Space),
                Effect::SetEmpty
            );
            // A space in a value is a space.
            assert_eq!(on_key(TEXT, false, true, Key::Space), Effect::Pass);
            assert_eq!(on_key(TEXT, false, false, Key::Space), Effect::Pass);
        }

        #[test]
        fn other_columns_ignore_the_keys() {
            for key in [Key::Backspace, Key::Space] {
                assert_eq!(on_key(NUM, false, true, key), Effect::Pass);
                assert_eq!(on_key(NUM, true, true, key), Effect::Pass);
            }
        }

        #[test]
        fn typing_leaves_null_and_emptying_keeps_the_state() {
            assert!(!after_change(TEXT, true, "a"));
            assert!(after_change(TEXT, true, ""));
            // An emptied (non-NULL) field is '' until backspace says otherwise.
            assert!(!after_change(TEXT, false, ""));
            assert!(!after_change(NUM, true, ""));
        }

        #[test]
        fn value_is_none_only_for_null() {
            assert_eq!(value(TEXT, true, ""), None);
            assert_eq!(value(TEXT, false, ""), Some(String::new()));
            assert_eq!(value(TEXT, false, " "), Some(" ".into()));
            assert_eq!(value(NUM, false, ""), None);
            assert_eq!(value(NUM, false, "  "), None);
            assert_eq!(value(NUM, false, "7"), Some("7".into()));
            let not_null_num = Shape {
                text_like: false,
                nullable: false,
            };
            assert_eq!(value(not_null_num, false, ""), Some(String::new()));
        }

        #[test]
        fn placeholder_shows_for_null_not_for_the_empty_string() {
            assert!(show_placeholder(TEXT, true, ""));
            assert!(!show_placeholder(TEXT, false, ""));
            assert!(!show_placeholder(TEXT, true, "x"));
            assert!(show_placeholder(NUM, false, ""));
        }

        #[test]
        fn load_keeps_null_and_empty_apart() {
            assert_eq!(load(TEXT, None), (true, String::new()));
            assert_eq!(load(TEXT, Some("")), (false, String::new()));
            assert_eq!(load(TEXT, Some("a")), (false, "a".into()));
            assert_eq!(load(NUM, None), (false, String::new()));
        }

        /// Walk a whole interaction: '' → Backspace → NULL → Space → '' → "x".
        #[test]
        fn the_whole_dance() {
            let (mut null, mut text) = load(TEXT, Some(""));
            assert_eq!(value(TEXT, null, &text), Some(String::new()));
            assert_eq!(
                on_key(TEXT, null, text.is_empty(), Key::Backspace),
                Effect::SetNull
            );
            null = true;
            assert_eq!(value(TEXT, null, &text), None);
            assert_eq!(
                on_key(TEXT, null, text.is_empty(), Key::Space),
                Effect::SetEmpty
            );
            null = false;
            assert_eq!(value(TEXT, null, &text), Some(String::new()));
            text.push('x');
            null = after_change(TEXT, null, &text);
            assert_eq!(value(TEXT, null, &text), Some("x".into()));
        }
    }
}

/// What a [`CellInput`] tells its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellInputEvent {
    /// The value changed (typed, picked, or NULL toggled by the rule).
    Changed,
    /// Enter.
    Commit,
    /// Escape.
    Cancel,
    /// Tab / Shift-Tab (inline editors only; a form lets the key through).
    Tab { back: bool },
    /// The `{ }` button (inline), or the preview of a JSON-routed field (form): edit this value
    /// in the JSON dialog.
    OpenJson,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flavor {
    Plain,
    Bool,
    Date,
    DateTime,
}

type BoolSelect = SelectState<SearchableVec<SharedString>>;

/// Width of the date picker beside the time field.
const PICKER_W_FORM: f32 = 128.0;
const PICKER_W_INLINE: f32 = 108.0;
/// The form's one-line JSON preview.
const PREVIEW_H: f32 = 26.0;

pub struct CellInput {
    shape: rule::Shape,
    /// What an empty NULL field shows: `NULL`, `NULL · default x`, `generated`…
    placeholder: SharedString,
    flavor: Flavor,
    inline: bool,
    disabled: bool,
    /// NULL, on a text-like column.
    null: bool,
    /// A `json` / `jsonb` column.
    json_col: bool,
    /// The JSON dialog can edit this column (`json`, or text-like): the `{ }` button is offered.
    json_capable: bool,
    /// The loaded value goes to the JSON dialog ([`json::route`]): a form field shows a preview
    /// to click instead of the single-line input. Decided by [`CellInput::load`].
    json: bool,
    /// The text field; for a date-time, the time-of-day field.
    input: Entity<InputState>,
    date: Option<Entity<DatePickerState>>,
    /// The boolean dropdown.
    select: Option<Entity<BoolSelect>>,
    /// Why a key did nothing ("NOT NULL").
    hint: Option<SharedString>,
    /// Why the value was refused; set by the owner.
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CellInputEvent> for CellInput {}

/// The placeholder of an empty field, from what the column says about itself.
pub fn placeholder_for(meta: &ColumnMeta) -> String {
    match (&meta.default, meta.nullable) {
        (Some(d), true) => format!("NULL · default {d}"),
        (Some(d), false) => format!("default {d}"),
        (None, true) => "NULL".to_string(),
        (None, false) if meta.auto_increment => "generated".to_string(),
        (None, false) => String::new(),
    }
}

impl CellInput {
    /// An input for `meta`'s column. `inline` is the grid's cell editor: flat, filling the cell,
    /// and it reports Enter / Escape / Tab (a boolean or date pick commits at once).
    pub fn new(
        meta: &ColumnMeta,
        inline: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let shape = rule::Shape {
            text_like: meta.data_type.is_text_like(),
            nullable: meta.nullable,
        };
        let flavor = match &meta.data_type {
            DataType::Bool => Flavor::Bool,
            DataType::Date => Flavor::Date,
            DataType::DateTime | DataType::DateTimeTz => Flavor::DateTime,
            _ => Flavor::Plain,
        };
        let input = cx.new(|cx| {
            let state = InputState::new(window, cx);
            if flavor == Flavor::DateTime {
                state.placeholder("HH:MM:SS")
            } else {
                state
            }
        });
        let mut subscriptions =
            vec![
                cx.subscribe(&input, |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => this.text_changed(cx),
                    InputEvent::PressEnter { .. } => cx.emit(CellInputEvent::Commit),
                    _ => {}
                }),
            ];
        let date = matches!(flavor, Flavor::Date | Flavor::DateTime).then(|| {
            let state = cx.new(|cx| DatePickerState::new(window, cx).date_format("%Y-%m-%d"));
            subscriptions.push(
                cx.subscribe(&state, move |this, _, _: &DatePickerEvent, cx| {
                    this.hint = None;
                    this.error = None;
                    cx.emit(CellInputEvent::Changed);
                    // A date column has nothing more to enter: inline, the pick is the commit.
                    if inline && flavor == Flavor::Date {
                        cx.emit(CellInputEvent::Commit);
                    }
                    cx.notify();
                }),
            );
            state
        });
        let select = (flavor == Flavor::Bool).then(|| {
            let items: Vec<SharedString> = bool_pick::items(meta.nullable)
                .into_iter()
                .map(SharedString::from)
                .collect();
            let state = cx.new(|cx| SelectState::new(SearchableVec::new(items), None, window, cx));
            subscriptions.push(cx.subscribe(
                &state,
                move |this, _, _: &SelectEvent<SearchableVec<SharedString>>, cx| {
                    this.hint = None;
                    this.error = None;
                    cx.emit(CellInputEvent::Changed);
                    // Inline, choosing is committing.
                    if inline {
                        cx.emit(CellInputEvent::Commit);
                    }
                    cx.notify();
                },
            ));
            state
        });
        Self {
            shape,
            placeholder: placeholder_for(meta).into(),
            flavor,
            inline,
            disabled: false,
            null: false,
            json_col: matches!(meta.data_type, DataType::Json),
            json_capable: json::can_force(&meta.data_type),
            json: matches!(meta.data_type, DataType::Json),
            input,
            date,
            select,
            hint: None,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    /// The current value as `parse_input` takes it: `None` = NULL.
    pub fn value(&self, cx: &gpui::App) -> Option<String> {
        let text = self.input.read(cx).value();
        let day = || {
            self.date
                .as_ref()
                .and_then(|d| d.read(cx).date().start())
                .map(|d| d.format("%Y-%m-%d").to_string())
        };
        match self.flavor {
            Flavor::Plain => rule::value(self.shape, self.null, &text),
            Flavor::Bool => {
                let choice = self
                    .select
                    .as_ref()
                    .and_then(|s| s.read(cx).selected_value().cloned());
                bool_pick::value(choice.as_deref())
            }
            Flavor::Date => day(),
            Flavor::DateTime => {
                let time = text.trim();
                match (day(), time.is_empty()) {
                    (None, true) => None,
                    (None, false) => Some(time.to_string()),
                    (Some(day), true) => Some(format!("{day} 00:00:00")),
                    (Some(day), false) => Some(format!("{day} {time}")),
                }
            }
        }
    }

    /// Show `value` (`None` = NULL) and forget any hint or error. Emits nothing.
    pub fn load(&mut self, value: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        self.hint = None;
        self.error = None;
        let (null, text) = rule::load(self.shape, value);
        self.null = null;
        self.json =
            self.json_col || (self.shape.text_like && value.is_some_and(json::looks_like_json));
        match (self.flavor, &self.date, &self.select) {
            (Flavor::Bool, _, Some(select)) => {
                let choice = bool_pick::choice(value, self.shape.nullable);
                select.update(cx, |select, cx| match choice {
                    Some(choice) => {
                        select.set_selected_value(&SharedString::from(choice), window, cx)
                    }
                    None => select.set_selected_index(None, window, cx),
                });
            }
            (Flavor::Plain | Flavor::Bool, _, _) | (_, None, _) => {
                self.input
                    .update(cx, |input, cx| input.set_value(text, window, cx));
            }
            (flavor, Some(date), _) => {
                let value = value.unwrap_or_default();
                let day = value
                    .get(..10)
                    .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
                date.update(cx, |date, cx| date.set_date(Date::Single(day), window, cx));
                let time = if flavor == Flavor::DateTime && day.is_some() {
                    value[10..].trim_start_matches(['T', ' ']).to_string()
                } else {
                    String::new()
                };
                self.input
                    .update(cx, |input, cx| input.set_value(time, window, cx));
            }
        }
        cx.notify();
    }

    /// Put the keyboard on the control that edits: the dropdown, the date picker, or the text
    /// field (for a date-time, the time field).
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        match (self.flavor, &self.select, &self.date) {
            (Flavor::Bool, Some(select), _) => {
                select.update(cx, |select, cx| select.focus(window, cx))
            }
            (Flavor::Date, _, Some(date)) => {
                let handle = date.focus_handle(cx);
                window.focus(&handle, cx);
            }
            _ => self.input.update(cx, |input, cx| input.focus(window, cx)),
        }
    }

    /// Open the dropdown or the calendar once the editor has been drawn (a boolean or date
    /// editor that has just been created). Neither state offers a public `open`, so this sends
    /// the Enter that opens a focused, closed one, on the next frame, when the control is in the
    /// tree to receive it; nothing is sent if focus went elsewhere in between.
    pub fn open_picker(&self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = match (self.flavor, &self.select, &self.date) {
            (Flavor::Bool, Some(select), _) => select.focus_handle(cx),
            (Flavor::Date, _, Some(date)) => date.focus_handle(cx),
            _ => return,
        };
        window.on_next_frame(move |window, cx| {
            if handle.is_focused(window)
                && let Ok(enter) = Keystroke::parse("enter")
            {
                window.dispatch_keystroke(enter, cx);
            }
        });
    }

    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        self.input
            .update(cx, |input, cx| input.set_disabled(disabled, cx));
        cx.notify();
    }

    /// Why the value was refused (shown red under the field / around the inline editor).
    pub fn set_error(&mut self, error: Option<String>, cx: &mut Context<Self>) {
        self.error = error.map(Into::into);
        cx.notify();
    }

    /// The loaded value is edited in the JSON dialog (a `json` column, or a text one holding a
    /// JSON object / array).
    pub fn is_json(&self) -> bool {
        self.json
    }

    /// A form field that edits in the JSON dialog: the value as a one-line preview to click.
    fn json_preview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let value = self.value(cx);
        div()
            .id("cell-json-preview")
            .flex()
            .items_center()
            .w_full()
            .min_w(px(0.))
            .h(px(theme::scaled(PREVIEW_H)))
            .px_2()
            .border(px(theme::hairline()))
            .border_color(theme::border())
            .bg(theme::surface_raised())
            .cursor_pointer()
            .hover(|style| style.border_color(theme::border_focus()))
            .font_family(theme::MONO_FONT)
            .text_size(theme::font(Family::Content, Role::Body))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .when(self.disabled, |this| this.text_color(theme::text_muted()))
            .when(value.is_none(), |this| {
                this.italic().text_color(theme::text_faint())
            })
            .on_click(
                cx.listener(|_, _: &gpui::ClickEvent, _, cx| cx.emit(CellInputEvent::OpenJson)),
            )
            .child(SharedString::from(match value {
                Some(text) => json::preview(&text, 200),
                None => "NULL".to_string(),
            }))
    }

    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    /// The reason the last key did nothing, if it did nothing.
    pub fn hint(&self) -> Option<&SharedString> {
        self.hint.as_ref()
    }

    fn text_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value();
        self.null = rule::after_change(self.shape, self.null, &text);
        self.hint = None;
        self.error = None;
        cx.emit(CellInputEvent::Changed);
        cx.notify();
    }

    /// Run the rule for `key`; true = the key is consumed.
    fn key(&mut self, key: rule::Key, cx: &mut Context<Self>) -> bool {
        let empty = self.input.read(cx).value().is_empty();
        match rule::on_key(self.shape, self.null, empty, key) {
            rule::Effect::Pass => return false,
            rule::Effect::SetNull => self.null = true,
            rule::Effect::SetEmpty => self.null = false,
            rule::Effect::Refuse => {
                self.hint = Some("NOT NULL — cannot be NULL".into());
                cx.notify();
                return true;
            }
        }
        self.hint = None;
        self.error = None;
        cx.emit(CellInputEvent::Changed);
        cx.notify();
        true
    }

    /// The text field with the rule's key handling and the NULL label.
    fn text_field(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let text = self.input.read(cx).value();
        let show_null = self.flavor == Flavor::Plain
            && rule::show_placeholder(self.shape, self.null, &text)
            && !self.placeholder.is_empty();
        let text_like = self.shape.text_like && self.flavor == Flavor::Plain;
        let mut input = Input::new(&self.input).small();
        if self.inline {
            // No chrome, no padding: the editor box pads like the cell, and the text is set in
            // the cell's own font so it does not change when editing starts.
            input = input
                .appearance(false)
                .px_0()
                .py_0()
                .font_family(theme::MONO_FONT)
                .text_size(theme::font(Family::Content, Role::Body));
        }
        div()
            .relative()
            .flex_1()
            .min_w(px(0.))
            .when(self.inline && self.flavor == Flavor::DateTime, |this| {
                this.px(px(theme::scaled(6.0)))
            })
            .when(text_like, |this| {
                this.capture_action(cx.listener(|this, _: &Backspace, _, cx| {
                    if this.key(rule::Key::Backspace, cx) {
                        cx.stop_propagation();
                    }
                }))
                .capture_key_down(cx.listener(
                    |this, event: &KeyDownEvent, _, cx| {
                        let m = event.keystroke.modifiers;
                        let plain = !(m.control || m.platform || m.alt);
                        if plain
                            && event.keystroke.key_char.as_deref() == Some(" ")
                            && this.key(rule::Key::Space, cx)
                        {
                            cx.stop_propagation();
                        }
                    },
                ))
            })
            .when(self.inline, |this| {
                this.capture_action(cx.listener(|_, _: &IndentInline, _, cx| {
                    cx.emit(CellInputEvent::Tab { back: false });
                    cx.stop_propagation();
                }))
                .capture_action(cx.listener(|_, _: &OutdentInline, _, cx| {
                    cx.emit(CellInputEvent::Tab { back: true });
                    cx.stop_propagation();
                }))
                .on_action(cx.listener(|_, _: &Escape, _, cx| cx.emit(CellInputEvent::Cancel)))
            })
            .child(input)
            .when(show_null, |this| {
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px(if self.inline {
                            0.0
                        } else {
                            theme::scaled(9.0)
                        }))
                        .flex()
                        .items_center()
                        .italic()
                        .when(self.inline, |this| this.font_family(theme::MONO_FONT))
                        .text_size(theme::font(Family::Content, Role::Body))
                        .text_color(theme::text_faint())
                        .child(self.placeholder.clone()),
                )
            })
    }
}

impl Render for CellInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let invalid = self.error.is_some();
        let picker = self.date.as_ref().map(|date| {
            let placeholder = if self.shape.nullable {
                "NULL"
            } else {
                "pick a date"
            };
            let mut picker = DatePicker::new(date)
                .small()
                .placeholder(placeholder)
                .cleanable(self.shape.nullable)
                .disabled(self.disabled);
            if self.inline {
                picker = picker.appearance(false);
            }
            div()
                .when(self.flavor == Flavor::Date, |this| {
                    this.flex_1().min_w(px(0.))
                })
                .when(self.flavor == Flavor::DateTime, |this| {
                    this.flex_none().w(px(theme::scaled(if self.inline {
                        PICKER_W_INLINE
                    } else {
                        PICKER_W_FORM
                    })))
                })
                .child(picker)
        });
        let dropdown = self.select.as_ref().map(|select| {
            let placeholder = if self.placeholder.is_empty() {
                SharedString::from("choose")
            } else {
                self.placeholder.clone()
            };
            let mut dropdown = Select::new(select)
                .small()
                .placeholder(placeholder)
                .disabled(self.disabled);
            if self.inline {
                dropdown = dropdown.appearance(false).border_0();
            }
            div().flex_1().min_w(px(0.)).child(dropdown)
        });
        let row = div().flex().flex_row().items_center().gap_1().w_full();
        let row = match self.flavor {
            Flavor::Plain if self.json && !self.inline => row.child(self.json_preview(cx)),
            Flavor::Plain => row.child(self.text_field(cx)),
            Flavor::Bool => row.children(dropdown),
            Flavor::Date => row.children(picker),
            Flavor::DateTime => row.children(picker).child(self.text_field(cx)),
        };
        if self.inline {
            // Exactly the cell: cancel the table's cell padding with negative insets, then pad
            // the same way inside. The dropdown and the picker bring their own horizontal
            // padding (the same as the cell's), so only the text field needs it here.
            let pad = Size::Medium.table_cell_padding();
            return div()
                .absolute()
                .top(-pad.top)
                .bottom(-pad.bottom)
                .left(-pad.left)
                .right(-pad.right)
                .flex()
                .items_center()
                .py(pad.top)
                .when(self.flavor == Flavor::Plain, |this| this.px(pad.left))
                .bg(theme::surface_raised())
                .child(row)
                // `{ }`: edit this text in the JSON dialog (the cell is committed first).
                .when(self.json_capable && self.flavor == Flavor::Plain, |this| {
                    this.child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right(px(theme::scaled(4.0)))
                            .flex()
                            .items_center()
                            .child(open_button("cell-json-open").on_click(cx.listener(
                                |_, _: &gpui::ClickEvent, _, cx| cx.emit(CellInputEvent::OpenJson),
                            ))),
                    )
                })
                // The outline is an overlay: it takes no room, so nothing shifts.
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .border_1()
                        .border_color(if invalid {
                            theme::danger()
                        } else {
                            theme::border_focus()
                        }),
                )
                .into_any_element();
        }
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(row)
            .when_some(self.hint.clone(), |this, hint| {
                this.child(
                    div()
                        .text_size(theme::font(Family::Content, Role::Dense))
                        .text_color(theme::warning())
                        .child(hint),
                )
            })
            .into_any_element()
    }
}
