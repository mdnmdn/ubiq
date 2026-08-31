//! The page every rail mode that is not built yet shows.

use gpui::{IntoElement, ParentElement, SharedString, Styled, div, px};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::theme;

pub fn empty_page(title: &str, note: &str, icon: IconName) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .items_center()
        .justify_center()
        .gap_3()
        .bg(theme::app_bg())
        .child(
            Icon::new(icon)
                .with_size(Size::Large)
                .text_color(theme::text_faint()),
        )
        .child(
            div()
                .text_size(px(15.))
                .text_color(theme::text())
                .child(SharedString::from(title.to_string())),
        )
        .child(
            div()
                .max_w(px(320.))
                .text_size(px(12.5))
                .text_color(theme::text_muted())
                .child(SharedString::from(note.to_string())),
        )
        .child(
            div()
                .text_size(px(11.))
                .font_family(theme::MONO_FONT)
                .text_color(theme::text_faint())
                .child("not built yet"),
        )
}
