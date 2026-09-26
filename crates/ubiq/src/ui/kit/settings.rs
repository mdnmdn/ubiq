//! Furniture the settings pages share: a heading, a label/control row, a hinted one, a nav item.
//!
//! **Nothing here is a new primitive.** The kitchen sink composed these first; the live settings
//! overlay and project settings use the same functions, so a row looked at on the sink is the row
//! a screen draws.

use gpui::{
    AnyElement, ClickEvent, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, px, relative,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::mono;

pub fn heading(title: &str, note: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .pb_2()
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Title))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .child(SharedString::from(title.to_string())),
        )
        .child(
            div()
                .max_w(px(560.))
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text_muted())
                .child(SharedString::from(note.to_string())),
        )
        .into_any_element()
}

/// The label column's floor. A question whose control is wide — a field-style picker, or four
/// pills naming an indexing level — used to squeeze this column to one character a line rather
/// than give way; below this width the control wraps onto its own line instead.
const LABEL_MIN_WIDTH: f32 = 220.0;

pub fn setting_row(label: &str, note: &str, control: AnyElement) -> AnyElement {
    div()
        .w(relative(1.))
        .py_3()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_x_6()
        .gap_y_2()
        .border_b_1()
        .border_color(theme::border())
        .child(
            div()
                .flex()
                .flex_1()
                .min_w(px(LABEL_MIN_WIDTH))
                .child(label_block(label, note)),
        )
        .child(control)
        .into_any_element()
}

/// The nav-and-body split **both** settings containers draw: a scrolling nav beside a scrolling
/// page (`D180` — the container owns layout and scroll, a section owns content).
///
/// One component rather than two near-identical ones, which is what had let the two drift into
/// different widths and different padding. Both halves scroll, so neither a nav longer than the
/// panel nor a section longer than it can grow the dialog — the bug `T-244` is about.
///
/// `prefix` is the container's own id prefix (`app-settings`, `project-settings`,
/// `sink-project`); the two scrollers take `{prefix}-nav` and `{prefix}-body`, because
/// `overflow_y_scroll` does nothing without an id.
pub fn settings_split(prefix: &str, items: Vec<AnyElement>, body: AnyElement) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .child(
            div()
                .id(ElementId::Name(format!("{prefix}-nav").into()))
                .w(px(theme::settings_nav_width()))
                .flex()
                .flex_none()
                .flex_col()
                .gap_1()
                .px_2()
                .py_3()
                .overflow_y_scroll()
                .bg(theme::pane_bg())
                .border_r_1()
                .border_color(theme::border())
                .children(items),
        )
        .child(
            div()
                .id(ElementId::Name(format!("{prefix}-body").into()))
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .overflow_y_scroll()
                .px_5()
                .py_4()
                .child(body),
        )
        .into_any_element()
}

/// A label with its explanation folded into a hint icon beside it, and a row built out of one.
///
/// [`label_block`] spends a whole line on the note, which is what turns a form of eight questions
/// into a form that scrolls. The words are worth having and worth reading **once**, so they move
/// onto the hover of a mark next to the label: the row stays one line high, and the explanation is
/// a pointer away rather than gone.
///
/// It takes an id because a tooltip needs a stateful element to hang off — the same bargain
/// [`crate::ui::kit::elided`] makes.
pub fn label_hint(id: impl Into<ElementId>, label: &str, hint: &str) -> AnyElement {
    let hint: SharedString = hint.to_string().into();
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text())
                .child(SharedString::from(label.to_string())),
        )
        .child(
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .child(
                    Icon::new(IconName::Info)
                        .with_size(Size::XSmall)
                        .text_color(theme::text_faint()),
                )
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(hint.clone()).build(window, cx)
                }),
        )
        .into_any_element()
}

/// One question on one line: the label and its hint on the left, the control on the right.
///
/// [`setting_row`]'s shape without the note under it, for a form dense enough that the note is a
/// hint — see [`label_hint`]. No rule between rows either: what groups these is the block they are
/// drawn in, and a hairline under every one of eight rows reads as eight sections.
pub fn hint_row(
    id: impl Into<ElementId>,
    label: &str,
    hint: &str,
    control: AnyElement,
) -> AnyElement {
    div()
        .w(relative(1.))
        .py_1()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .child(label_hint(id, label, hint))
        .child(control)
        .into_any_element()
}

pub fn label_block(label: &str, note: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_1()
        .min_w(px(0.))
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text())
                .child(SharedString::from(label.to_string())),
        )
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_muted())
                .child(SharedString::from(note.to_string())),
        )
        .into_any_element()
}

pub fn nav_item(
    id: impl Into<ElementId>,
    icon: impl Into<Icon>,
    label: &str,
    count: Option<usize>,
    selected: bool,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let fg = if !enabled {
        theme::text_faint()
    } else if selected {
        theme::text()
    } else {
        theme::text_muted()
    };
    let icon_fg = if !enabled {
        theme::text_faint()
    } else if selected {
        theme::accent()
    } else {
        theme::text_muted()
    };

    let mut row = div()
        .id(id)
        .h(px(32.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .child(Icon::new(icon).with_size(Size::Small).text_color(icon_fg))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(fg)
                .child(SharedString::from(label.to_string())),
        );

    if let Some(count) = count {
        row = row.child(
            mono(format!("{count}"), theme::text_faint())
                .text_size(theme::font(Family::Chrome, Role::Meta)),
        );
    }
    if selected && enabled {
        row = row
            .bg(theme::accent_soft())
            .border_l(px(theme::accent_edge()))
            .border_color(theme::accent());
    }

    if enabled {
        row = row
            .cursor_pointer()
            .hover(|this| this.bg(theme::hover()))
            .on_click(on_click);
    }

    row.into_any_element()
}

pub fn column(children: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .w(relative(1.))
        .children(children)
        .into_any_element()
}
