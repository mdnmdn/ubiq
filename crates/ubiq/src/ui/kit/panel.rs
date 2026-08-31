//! Panel chrome: the bordered surfaces, their headers, and the tab strip the editor and the
//! terminal dock both use.

use std::rc::Rc;

use gpui::{
    AnyElement, App, Div, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Rgba, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::theme;
use crate::ui::kit::IndexedAction;
use crate::ui::kit::controls::section_label;

/// A panel column: a surface with one border against its neighbour.
pub fn panel() -> Div {
    div()
        .flex()
        .flex_col()
        .size_full()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::pane_bg())
}

/// The row at the top of a panel: an uppercase title, then whatever actions the panel offers.
pub fn panel_header(title: &str, actions: impl IntoElement) -> impl IntoElement {
    div()
        .h(px(38.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap_2()
        .child(section_label(title))
        .child(div().flex().items_center().gap_1().child(actions))
}

/// One entry in a tab strip.
pub struct Tab {
    pub label: SharedString,
    pub dot: Option<Rgba>,
    pub closable: bool,
}

impl Tab {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            dot: None,
            closable: false,
        }
    }

    pub fn dot(mut self, colour: Rgba) -> Self {
        self.dot = Some(colour);
        self
    }

    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }
}

/// The shared tab strip. The active tab is marked on its bottom edge, which is the one place the
/// editor and the dock agree to put it.
pub fn tab_strip(
    id: &'static str,
    tabs: Vec<Tab>,
    active: usize,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
    on_close: Option<IndexedAction>,
    trailing: Option<AnyElement>,
) -> impl IntoElement {
    let select = Rc::new(on_select);

    let rendered: Vec<_> = tabs
        .into_iter()
        .enumerate()
        .map(|(ix, tab)| {
            let is_active = ix == active;
            let select = select.clone();
            let close = on_close.clone();

            let mut row = div()
                .id((id, ix))
                .h(px(38.))
                .px_3()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .border_b_2()
                .border_color(if is_active {
                    theme::accent()
                } else {
                    theme::border()
                })
                .text_size(px(12.5))
                .text_color(if is_active {
                    theme::text()
                } else {
                    theme::text_muted()
                })
                .cursor_pointer()
                .hover(|this| this.text_color(theme::text()));

            if is_active {
                row = row.bg(theme::app_bg());
            }

            if let Some(colour) = tab.dot {
                row = row.child(div().size(px(7.)).flex_none().rounded_full().bg(colour));
            }

            row = row.child(tab.label.clone());

            if tab.closable {
                row = row.child(
                    div()
                        .id(ElementId::Name(format!("{id}-close-{ix}").into()))
                        .size(px(16.))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|this| this.bg(theme::hover()))
                        .child(
                            Icon::new(IconName::Close)
                                .with_size(Size::XSmall)
                                .text_color(theme::text_faint()),
                        )
                        .on_click(move |_, window, cx| {
                            if let Some(close) = close.clone() {
                                close(ix, window, cx);
                            }
                        }),
                );
            }

            row.on_click(move |_, window, cx| select(ix, window, cx))
        })
        .collect();

    let mut strip = div()
        .h(px(38.))
        .flex()
        .flex_none()
        .items_center()
        .bg(theme::pane_bg())
        .border_b_1()
        .border_color(theme::border())
        .font_weight(FontWeight::NORMAL)
        .children(rendered);

    if let Some(trailing) = trailing {
        strip = strip.child(div().flex_1().min_w(px(0.))).child(
            div()
                .px_2()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .child(trailing),
        );
    }

    strip
}
