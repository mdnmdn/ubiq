//! Furniture the settings pages share: a heading, a label/control row, a nav item.
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
use crate::ui::kit::mono;

pub fn heading(title: &str, note: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .pb_2()
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .child(SharedString::from(title.to_string())),
        )
        .child(
            div()
                .max_w(px(560.))
                .text_size(px(12.5))
                .text_color(theme::text_muted())
                .child(SharedString::from(note.to_string())),
        )
        .into_any_element()
}

pub fn setting_row(label: &str, note: &str, control: AnyElement) -> AnyElement {
    div()
        .w(relative(1.))
        .py_3()
        .flex()
        .items_center()
        .justify_between()
        .gap_6()
        .border_b_1()
        .border_color(theme::border())
        .child(label_block(label, note))
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
                .text_size(px(13.5))
                .text_color(theme::text())
                .child(SharedString::from(label.to_string())),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(theme::text_muted())
                .child(SharedString::from(note.to_string())),
        )
        .into_any_element()
}

pub fn nav_item(
    id: impl Into<ElementId>,
    icon: IconName,
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
                .text_size(px(12.5))
                .text_color(fg)
                .child(SharedString::from(label.to_string())),
        );

    if let Some(count) = count {
        row = row.child(mono(format!("{count}"), theme::text_faint()).text_size(px(11.)));
    }
    if selected && enabled {
        row = row
            .bg(theme::accent_soft())
            .border_l(px(theme::ACCENT_EDGE))
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
