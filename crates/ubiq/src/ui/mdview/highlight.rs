//! **Block highlights, as the page draws them:** a colour dot in the left margin of every row the
//! host's decor gives a highlight, in every mode that draws blocks, and the `●` chip's colour menu.
//!
//! The view owns no highlight. A dot is drawn from the [`RowDecor`](crate::state::document::RowDecor)
//! the host pushed (`MdView::set_decor`), and a pick from the menu is an
//! [`MdViewEvent::HighlightRequested`](super::events::MdViewEvent::HighlightRequested) the host acts
//! on and answers with fresh decor. Colours are `theme::highlight_ink` — derived, never per palette.
//!
//! Ported from the markdown-viewer spike's `ui/highlight.rs` `dot` and `colour_menu` (mission T-298).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, ClickEvent, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, px,
};
use ubiq_proto::plan::HighlightColour;

use super::annotation::{self, RowMenu};
use super::blocks::Metrics;
use super::view::MdView;
use crate::theme;

/// Every highlight colour, in the order the menu lists them.
pub const COLOURS: [HighlightColour; 5] = [
    HighlightColour::Yellow,
    HighlightColour::Green,
    HighlightColour::Blue,
    HighlightColour::Red,
    HighlightColour::Purple,
];

/// The word a colour is listed under.
pub fn label(colour: HighlightColour) -> &'static str {
    match colour {
        HighlightColour::Yellow => "Yellow",
        HighlightColour::Green => "Green",
        HighlightColour::Blue => "Blue",
        HighlightColour::Red => "Red",
        HighlightColour::Purple => "Purple",
    }
}

/// Where the dot's left edge sits: half a margin left of the text column, never nearer the row's
/// edge than the selection bar plus a gap.
pub fn dot_left(m: &Metrics) -> f32 {
    (m.inset - m.margin * 0.5 - theme::MD_HIGHLIGHT_DOT * 0.5).max(theme::ACCENT_EDGE + 4.0)
}

/// **The left-margin decorator.** Absolute, so the row's measured height is untouched, and
/// vertically centred in the row. A click selects the block and reveals it.
pub fn dot(
    view: &WeakEntity<MdView>,
    ix: usize,
    m: &Metrics,
    colour: HighlightColour,
) -> AnyElement {
    let click = view.clone();
    div()
        .absolute()
        .top_0()
        .h_full()
        .left(px(dot_left(m)))
        .flex()
        .items_center()
        .child(
            div()
                .id(("md-highlight", ix))
                .debug_selector(move || format!("md-highlight-{ix}"))
                .size(px(theme::MD_HIGHLIGHT_DOT))
                .bg(theme::highlight_ink(colour))
                .cursor_pointer()
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(format!("Highlighted {}", label(colour)))
                        .build(window, cx)
                })
                .on_click(move |_: &ClickEvent, _window, cx: &mut App| {
                    cx.stop_propagation();
                    click
                        .update(cx, |this, cx| {
                            this.select_block(ix, cx);
                            this.reveal_block(ix, cx);
                        })
                        .ok();
                }),
        )
        .into_any_element()
}

/// The `●` chip and, while open, its menu: one entry per colour (its swatch and name, lit for the
/// row's current colour) and `None` to clear. The chip is inked in the current colour.
pub fn chip(
    view: &WeakEntity<MdView>,
    ix: usize,
    current: Option<HighlightColour>,
    open: bool,
) -> AnyElement {
    div()
        .relative()
        .child(
            annotation::menu_chip(
                view,
                ("md-highlight-open", ix),
                "●",
                "Highlight",
                RowMenu::Highlight(ix),
                open,
            )
            .when_some(current, |this, colour| {
                this.text_color(theme::highlight_ink(colour))
            }),
        )
        .children(open.then(|| colour_menu(view, ix, current)))
        .into_any_element()
}

fn colour_menu(
    view: &WeakEntity<MdView>,
    ix: usize,
    current: Option<HighlightColour>,
) -> AnyElement {
    let entry = |i: usize, colour: Option<HighlightColour>| {
        let pick = view.clone();
        let swatch = div()
            .size(px(theme::MD_HIGHLIGHT_DOT))
            .flex_none()
            .map(|this| match colour {
                Some(colour) => this.bg(theme::highlight_ink(colour)),
                None => this.border_1().border_color(theme::border()),
            });
        annotation::menu_entry(
            ("md-highlight-item", i),
            colour.map_or("None", label),
            colour.is_some() && current == colour,
            Some(swatch.into_any_element()),
            move |_: &ClickEvent, _window, cx: &mut App| {
                cx.stop_propagation();
                pick.update(cx, |this, cx| this.request_highlight(ix, colour, cx))
                    .ok();
            },
        )
        // The current colour's entry is lit in its own tint rather than the accent's.
        .when_some(colour.filter(|c| current == Some(*c)), |this, colour| {
            this.bg(theme::highlight(colour))
        })
        .into_any_element()
    };
    let entries = COLOURS
        .into_iter()
        .enumerate()
        .map(|(i, colour)| entry(i, Some(colour)))
        .chain(std::iter::once(entry(COLOURS.len(), None)));
    annotation::menu_panel(view, ("md-highlight-menu", ix), entries)
}
