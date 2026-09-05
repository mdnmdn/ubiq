//! One dropdown mechanism, used by every menu in the window.
//!
//! The trigger renders in place and the list is painted through `deferred`, so it sits above the
//! rest of the shell and is dismissed by a click anywhere outside it. Which menu is open is the
//! caller's state, not the picker's — exactly one may be down at a time.

use std::rc::Rc;

use gpui::{
    Anchor, AnyElement, App, ElementId, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Pixels, Point, RenderOnce, SharedString, StatefulInteractiveElement, Styled,
    Window, anchored, deferred, div, px,
};
use gpui_component::input::{Input, InputState};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::theme;
use crate::ui::kit::{Action, IndexedAction, field};

/// Anchor for a menu that must open upward, clear of the window's bottom edge.
pub const MENU_ANCHOR_UP: Anchor = Anchor::BottomLeft;

/// How the trigger is drawn. The titlebar's pickers read as plain text; the composer's read as
/// chips, because they sit on a busier surface.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PickerStyle {
    Plain,
    Chip,
}

#[derive(IntoElement)]
pub struct Picker {
    id: ElementId,
    icon: Option<IconName>,
    label: SharedString,
    items: Vec<SharedString>,
    selected: Option<usize>,
    open: bool,
    anchor: Anchor,
    style: PickerStyle,
    on_toggle: Option<Action>,
    on_pick: Option<IndexedAction>,
    on_dismiss: Option<Action>,
    /// A filter field drawn at the top of the panel: the buffer, and whether it holds focus.
    /// `None` is every picker that has not opted in — see [`Self::search`].
    search: Option<(Entity<InputState>, bool)>,
}

impl Picker {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: None,
            label: label.into(),
            items: Vec::new(),
            selected: None,
            open: false,
            anchor: Anchor::TopLeft,
            style: PickerStyle::Plain,
            on_toggle: None,
            on_pick: None,
            on_dismiss: None,
            search: None,
        }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn items<S: AsRef<str>>(mut self, items: impl IntoIterator<Item = S>) -> Self {
        self.items = items
            .into_iter()
            .map(|s| SharedString::from(s.as_ref().to_string()))
            .collect();
        self
    }

    pub fn selected(mut self, index: usize) -> Self {
        self.selected = Some(index);
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Where the list hangs from the trigger. `BottomLeft` opens upward, which is what the
    /// composer needs.
    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn style(mut self, style: PickerStyle) -> Self {
        self.style = style;
        self
    }

    pub fn on_toggle(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(handler));
        self
    }

    pub fn on_pick(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_pick = Some(Rc::new(handler));
        self
    }

    pub fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }

    /// Draw a filter field at the top of the panel. The caller has already filtered `items`;
    /// this only draws the field and reports the focus ring — the same split `project_menu`'s
    /// hand-rolled search uses.
    pub fn search(mut self, state: &Entity<InputState>, focused: bool) -> Self {
        self.search = Some((state.clone(), focused));
        self
    }
}

impl RenderOnce for Picker {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Picker {
            id,
            icon,
            label,
            items,
            selected,
            open,
            anchor,
            style,
            on_toggle,
            on_pick,
            on_dismiss,
            search,
        } = self;

        let panel_id = ElementId::Name(format!("{id:?}-menu").into());

        let mut trigger = div()
            .id(id)
            .relative()
            .h(px(26.))
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .text_size(px(13.))
            .text_color(theme::text())
            .cursor_pointer()
            .hover(|this| this.bg(theme::hover()));

        if style == PickerStyle::Chip {
            trigger = trigger
                .px_2()
                .bg(theme::surface())
                .border_l(px(theme::ACCENT_EDGE))
                .border_color(theme::border())
                .text_size(px(12.5));
        } else {
            trigger = trigger.px_2();
        }

        if let Some(icon) = icon {
            trigger = trigger.child(
                Icon::new(icon)
                    .with_size(Size::XSmall)
                    .text_color(theme::text_muted()),
            );
        }

        trigger = trigger.child(label).child(
            Icon::new(IconName::ChevronDown)
                .with_size(Size::XSmall)
                .text_color(theme::text_faint()),
        );

        if let Some(toggle) = on_toggle.clone() {
            // Opening rather than toggling: the panel's own outside-click dismissal would
            // otherwise race this click into reopening a menu the user meant to close.
            trigger = trigger.on_click(move |_, window, cx| toggle(window, cx));
        }

        if open {
            trigger = trigger.child(menu_panel(
                panel_id, anchor, items, selected, on_pick, on_dismiss, search,
            ));
        }

        trigger
    }
}

fn menu_panel(
    id: ElementId,
    anchor: Anchor,
    items: Vec<SharedString>,
    selected: Option<usize>,
    on_pick: Option<IndexedAction>,
    on_dismiss: Option<Action>,
    search: Option<(Entity<InputState>, bool)>,
) -> impl IntoElement {
    // The caller has already filtered `items` — an empty result is said, once, rather than left
    // as a panel with nothing in it.
    let rows: Vec<AnyElement> = if items.is_empty() {
        vec![
            div()
                .h(px(28.))
                .px_2()
                .flex()
                .items_center()
                .text_size(px(12.5))
                .text_color(theme::text_faint())
                .child("No matches")
                .into_any_element(),
        ]
    } else {
        items
            .into_iter()
            .enumerate()
            .map(|(ix, item)| {
                let is_selected = selected == Some(ix);
                let pick = on_pick.clone();
                div()
                    .id(("menu-row", ix))
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.5))
                    .text_color(if is_selected {
                        theme::text()
                    } else {
                        theme::text_muted()
                    })
                    .cursor_pointer()
                    .hover(|this| this.bg(theme::hover()).text_color(theme::text()))
                    .child(div().w(px(12.)).flex().flex_none().justify_center().child(
                        if is_selected {
                            Icon::new(IconName::Check)
                                .with_size(Size::XSmall)
                                .text_color(theme::accent())
                                .into_any_element()
                        } else {
                            div().into_any_element()
                        },
                    ))
                    .child(item)
                    .on_click(move |_, window, cx| {
                        if let Some(pick) = pick.clone() {
                            pick(ix, window, cx);
                        }
                    })
                    .into_any_element()
            })
            .collect()
    };

    let search_field = search.map(|(state, focused)| {
        field(theme::accent(), focused)
            .h(px(28.))
            .px_2()
            .flex_none()
            .gap_2()
            .border_b_1()
            .border_color(theme::border())
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
                    .child(Input::new(&state).appearance(false)),
            )
    });

    deferred(
        anchored()
            .anchor(anchor)
            .snap_to_window_with_margin(px(8.))
            .child(
                div()
                    .id(id)
                    .min_w(px(180.))
                    .p_1()
                    .flex()
                    .flex_col()
                    .bg(theme::surface_raised())
                    .border_l(px(theme::ACCENT_EDGE))
                    .border_color(theme::accent())
                    .shadow_lg()
                    .font_weight(FontWeight::NORMAL)
                    .children(search_field)
                    .children(rows)
                    .on_mouse_down_out(move |_, window, cx| {
                        if let Some(dismiss) = on_dismiss.clone() {
                            dismiss(window, cx);
                        }
                    }),
            ),
    )
    .priority(1)
}

/// One row in a context menu. `enabled` is what a prepared action that has nothing behind it yet
/// looks like: the wording is there, the click is not.
#[derive(Clone)]
pub struct ContextItem {
    pub label: SharedString,
    pub enabled: bool,
    /// A hairline instead of a row. It still occupies an index, because a menu's rows and the
    /// actions behind them are matched by position.
    pub separator: bool,
}

impl ContextItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            enabled: true,
            separator: false,
        }
    }

    /// Drawn, and does nothing: the predisposition for an action the host does not answer yet.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// The line between two groups of rows — what tells "start this" from "show me that".
    pub fn separator() -> Self {
        Self {
            label: SharedString::default(),
            enabled: false,
            separator: true,
        }
    }
}

/// A menu that opens where the pointer went down, not from a trigger.
///
/// Painted through `deferred` and `anchored` so a right-click inside a dock panel covers the window
/// rather than being clipped to the panel. Dismissal is an outside click, the same as the
/// dropdown: the two behave the same way and neither uses a scrim.
pub fn context_menu(
    id: impl Into<ElementId>,
    at: Point<Pixels>,
    items: Vec<ContextItem>,
    on_pick: impl Fn(usize, &mut Window, &mut App) + 'static,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id = id.into();
    deferred(
        anchored()
            .position(at)
            .snap_to_window_with_margin(px(8.))
            .child(context_panel(
                id,
                items,
                Some(Rc::new(on_pick)),
                Some(Rc::new(on_dismiss)),
            )),
    )
    .priority(1)
}

/// The panel a context menu is. Exported so the style reference can draw one in place, without
/// the deferred layer a real right-click needs.
pub fn context_panel(
    id: impl Into<ElementId>,
    items: Vec<ContextItem>,
    on_pick: Option<IndexedAction>,
    on_dismiss: Option<Action>,
) -> impl IntoElement {
    let rows: Vec<_> = items
        .into_iter()
        .enumerate()
        .map(|(ix, item)| {
            if item.separator {
                return div()
                    .id(("menu-separator", ix))
                    .my_1()
                    .h(px(1.))
                    .flex_none()
                    .bg(theme::border());
            }
            let pick = on_pick.clone();
            let enabled = item.enabled;
            let mut row = div()
                .id(("menu-row", ix))
                .h(px(28.))
                .px_2()
                .flex()
                .items_center()
                .text_size(px(12.5))
                .text_color(if enabled {
                    theme::text()
                } else {
                    theme::text_faint()
                })
                .child(item.label);

            if enabled {
                row = row
                    .cursor_pointer()
                    .hover(|this| this.bg(theme::hover()).text_color(theme::text()));
                if let Some(pick) = pick {
                    row = row.on_click(move |_, window, cx| pick(ix, window, cx));
                }
            }

            row
        })
        .collect();

    div()
        .id(id)
        .min_w(px(180.))
        .p_1()
        .flex()
        .flex_col()
        .bg(theme::surface_raised())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::accent())
        .shadow_lg()
        .font_weight(FontWeight::NORMAL)
        .children(rows)
        .on_mouse_down_out(move |_, window, cx| {
            if let Some(dismiss) = on_dismiss.clone() {
                dismiss(window, cx);
            }
        })
}
