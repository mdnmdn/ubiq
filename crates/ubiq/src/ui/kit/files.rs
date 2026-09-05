//! Shared chrome for a file list: the picker and the explorer draw the same row.
//!
//! **A row is one line, always.** A name that does not fit is elided; nothing here wraps, because
//! a wrapped row would push the rest of the list down and a list is scanned by its left edge. The
//! indent is drawn rather than padded, so a selected row's accent bar stays flush left.
//!
//! The picker and the explorer are two arrangements of this chrome: the picker ticks and confirms,
//! the explorer tints and badges. Colour, a leading mark and whatever sits at the far end are the
//! caller's — that is what lets git state land on an explorer row without the picker learning
//! version control.

use gpui::{
    App, ClickEvent, Div, ElementId, InteractiveElement, IntoElement, ParentElement, Rgba,
    Stateful, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::theme;
use crate::ui::kit::controls::{field, icon_button};

/// The text size a file list draws at when nobody scales it — the picker and the ref list, which
/// are dialogs rather than a project's workspace.
pub const ROW_FONT: f32 = 12.5;

/// How tall one row is at a given text size.
///
/// **A row is sized from its text, not from a constant.** The explorer scales with the project's
/// font, so a row that kept a fixed height would leave a gap around small text and clip large
/// text. The floor keeps the twisty and the kind icon from touching the edges; the ceiling stops
/// the tree turning into a list of buttons at the top of the range.
pub fn row_height(font_size: f32) -> f32 {
    (font_size * 1.7).round().clamp(18.0, 52.0)
}

/// How far each level of the tree indents, at a given text size. It scales with the row for the
/// same reason the height does — an indent is read against the text beside it.
pub fn row_indent(font_size: f32) -> f32 {
    (font_size * 0.85).round().clamp(8.0, 24.0)
}

/// The two-arrangement toggle: tree on the left, list on the right, lit when it is the one on
/// screen.
pub fn view_switch(
    tree_id: &'static str,
    list_id: &'static str,
    tree: bool,
    on_tree: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_list: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .border_1()
        .border_color(theme::border())
        .child(icon_button(tree_id, IconName::PanelLeft, tree, on_tree))
        .child(icon_button(list_id, IconName::Menu, !tree, on_list))
}

/// The filter field's surface: a search mark, the field, and whatever the caller puts at the end
/// — a prefilter chip, the name of the arrangement, a shortcut.
///
/// One field over both views on purpose: what was typed survives the toggle, because a user who
/// cannot find something in the tree switches to the list to look for the same thing.
///
/// `focused` is the shared text-field treatment — the surface, the left edge, and the focused
/// underline — so the picker and the explorer draw their filter the way every other field does.
pub fn filter_bar(
    input: impl IntoElement,
    trailing: impl IntoElement,
    focused: bool,
) -> impl IntoElement {
    // Flush with the panel's edges: the field is chrome, not content, so it touches the borders
    // rather than floating inside a margin.
    div().flex().flex_none().child(
        field(theme::border(), focused)
            .w_full()
            .h(px(28.))
            .px_2()
            .gap_1p5()
            .child(
                Icon::new(IconName::Search)
                    .with_size(Size::XSmall)
                    .text_color(theme::text_faint()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .text_size(px(12.5))
                    .child(input),
            )
            .child(trailing),
    )
}

/// One visible line, already indented and already marked for selection and the keyboard.
///
/// Two marks, and they say different things: the accent is what is chosen, and the keyboard's own
/// bar is only where the next key lands. A row that is both keeps the accent — what is chosen
/// outranks where the cursor happens to be — and takes the focus colour's edge to say the keyboard
/// is there too. `focused` is whether the list itself holds the keyboard, which is what deepens
/// the cursor bar.
pub fn file_row(
    id: impl Into<ElementId>,
    depth: usize,
    selected: bool,
    on_cursor: bool,
    focused: bool,
    font_size: f32,
) -> Stateful<Div> {
    let mut line = div()
        .id(id)
        .h(px(row_height(font_size)))
        .pr_1p5()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(
            div()
                .w(px(6.0 + depth as f32 * row_indent(font_size)))
                .flex_none(),
        );

    line = match (selected, on_cursor) {
        (true, false) => line
            .bg(theme::accent_soft())
            .border_l_2()
            .border_color(theme::accent()),
        (true, true) => line
            .bg(theme::accent_soft())
            .border_l_2()
            .border_color(theme::border_focus()),
        // The cursor bar deepens once the list holds the keyboard, so where the next key lands is
        // told apart from where a list left its cursor while the keyboard was somewhere else.
        (false, true) => line
            .bg(match focused {
                true => theme::selected_focus(),
                false => theme::selected(),
            })
            .border_l_2()
            .border_color(theme::border_focus()),
        (false, false) => line,
    };

    line
}

/// The chevron that opens a folder. Its own click target, so a row that *can* be chosen is opened
/// by this and chosen by the rest of itself.
pub fn twisty(
    id: impl Into<ElementId>,
    expanded: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .size(px(16.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .child(
            Icon::new(match expanded {
                true => IconName::ChevronDown,
                false => IconName::ChevronRight,
            })
            .with_size(Size::XSmall)
            .text_color(theme::text_muted()),
        )
        .on_click(on_click)
}

/// The folder-or-file mark a row leads with. Colour is the caller's: muted by default, a git
/// token when the explorer has something to say.
pub fn kind_icon(is_dir: bool, color: Rgba) -> impl IntoElement {
    Icon::new(match is_dir {
        true => IconName::Folder,
        false => IconName::File,
    })
    .with_size(Size::XSmall)
    .text_color(color)
}
