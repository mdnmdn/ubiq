//! A dropdown that lists a markdown document's headings, hierarchically, each beside how many
//! threads sit under it — open against settled.
//!
//! `crate::ui::plan` is the first caller. A second annotated document —
//! `crate::state::document`'s own open question — reuses this rather than growing its own list,
//! which is the whole reason it lives here rather than inline in `ui/plan.rs`.
//!
//! Built directly on the anchored-list device every dropdown in the window uses
//! (`deferred(anchored())`, [`super::menu::menu_panel`]'s own shape) rather than on
//! [`super::menu::Picker`]: a heading row carries an indent and two counts a picker's plain-label
//! rows have no place for, so this owns its row instead of forcing them through one.

use gpui::{
    AnyElement, App, Div, ElementId, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Rgba, SharedString, Stateful, StatefulInteractiveElement, Styled, Window,
    anchored, deferred, div, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::menu::{MENU_LAYER, MODAL_MENU_LAYER};
use crate::ui::kit::{IndexedAction, elided, status_dot};

/// One heading a navigator lists — a fact about the document, not a widget's own state. Built by
/// the caller from whatever it indexes a document's blocks and threads by;
/// `crate::state::document::heading_sections` is `crate::ui::plan`'s own source for it.
#[derive(Clone, Debug)]
pub struct MdNavEntry {
    /// `1` for `#`, up to `6` — indentation only, never a colour or a size of its own.
    pub level: u8,
    pub label: SharedString,
    pub open: usize,
    pub resolved: usize,
}

impl MdNavEntry {
    pub fn new(level: u8, label: impl Into<SharedString>, open: usize, resolved: usize) -> Self {
        Self {
            level,
            label: label.into(),
            open,
            resolved,
        }
    }
}

const ROW_HEIGHT: f32 = 28.0;
const INDENT_STEP: f32 = 14.0;
const PANEL_WIDTH: f32 = 280.0;
const PANEL_MAX_HEIGHT: f32 = 360.0;

/// The trigger — whatever the caller wants it to say, e.g. a running count of open threads — and,
/// while `open`, the list beneath it.
///
/// `on_select` is handed the row's own index into `entries`, the caller's to resolve back to
/// whatever it indexed the document by — the same discipline every indexed row in the window
/// keeps, bridged in from the view with `crate::ui::indexed`.
///
/// `above_modal` is [`crate::ui::kit::menu::Picker::above_modal`]'s own reason: a caller whose
/// trigger sits inside a modal (the plan editor's chrome strip is one) sets it so the panel paints
/// at [`MODAL_MENU_LAYER`] instead of [`MENU_LAYER`], over the modal's own overlay rather than
/// under it.
#[allow(clippy::too_many_arguments)]
pub fn md_navigator(
    id: impl Into<ElementId>,
    trigger: impl Into<SharedString>,
    open: bool,
    entries: &[MdNavEntry],
    above_modal: bool,
    on_toggle: impl Fn(&mut Window, &mut App) + 'static,
    on_select: IndexedAction,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let id = id.into();
    let panel_id = ElementId::Name(format!("{id:?}-panel").into());
    let layer = if above_modal {
        MODAL_MENU_LAYER
    } else {
        MENU_LAYER
    };

    // Opening rather than toggling — `Picker`'s own reason: the panel's outside-click dismissal
    // would otherwise race this click into reopening what the user meant to close.
    let mut root = trigger_row(id, trigger).on_click(move |_, window, cx| on_toggle(window, cx));
    if open {
        root = root.child(panel(panel_id, entries, layer, on_select, on_dismiss));
    }
    root.into_any_element()
}

fn trigger_row(id: ElementId, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .h(px(ROW_HEIGHT))
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .px_2()
        .cursor_pointer()
        .text_size(theme::font(Family::Chrome, Role::Meta))
        .text_color(theme::text_muted())
        .hover(|this| this.bg(theme::hover()).text_color(theme::text()))
        .child(label.into())
        .child(
            Icon::new(IconName::ChevronDown)
                .with_size(Size::XSmall)
                .text_color(theme::text_faint()),
        )
}

fn panel(
    id: ElementId,
    entries: &[MdNavEntry],
    layer: usize,
    on_select: IndexedAction,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let rows: Vec<AnyElement> = if entries.is_empty() {
        vec![empty_row()]
    } else {
        entries
            .iter()
            .enumerate()
            .map(|(index, entry)| row(index, entry, on_select.clone()))
            .collect()
    };

    deferred(
        anchored().snap_to_window_with_margin(px(8.)).child(
            div()
                .id(id)
                .w(px(PANEL_WIDTH))
                .max_h(px(PANEL_MAX_HEIGHT))
                .p_1()
                .flex()
                .flex_col()
                .overflow_y_scroll()
                .bg(theme::surface_raised())
                .border_l(px(theme::accent_edge()))
                .border_color(theme::accent())
                .shadow_lg()
                .font_weight(FontWeight::NORMAL)
                .children(rows)
                // Same reason as every other anchored list's panel: painted above whatever raised
                // it, so a click here must not also land on a control underneath.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down_out(move |_, window, cx| on_dismiss(window, cx)),
        ),
    )
    .priority(layer)
}

fn empty_row() -> AnyElement {
    div()
        .h(px(ROW_HEIGHT))
        .px_2()
        .flex()
        .items_center()
        .text_size(theme::font(Family::Chrome, Role::Body))
        .text_color(theme::text_faint())
        .child("No headings yet")
        .into_any_element()
}

fn row(index: usize, entry: &MdNavEntry, on_select: IndexedAction) -> AnyElement {
    let indent = px(INDENT_STEP * f32::from(entry.level.saturating_sub(1)));
    div()
        .id(("md-nav-row", index))
        .h(px(ROW_HEIGHT))
        .pl(indent + px(8.))
        .pr_2()
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(elided(
            ("md-nav-label", index),
            entry.label.clone(),
            theme::text(),
            theme::font(Family::Chrome, Role::Body),
        ))
        .children(counts(entry))
        .on_click(move |_, window, cx| on_select(index, window, cx))
        .into_any_element()
}

/// The two counts a row carries — open against settled — each only where it is not zero: a
/// section with nothing to say about its threads says nothing, rather than "0 · 0" on every row.
fn counts(entry: &MdNavEntry) -> Option<AnyElement> {
    if entry.open == 0 && entry.resolved == 0 {
        return None;
    }
    let mut node = div().flex().flex_none().items_center().gap_1p5();
    if entry.open > 0 {
        node = node.child(count_chip(entry.open, theme::info()));
    }
    if entry.resolved > 0 {
        node = node.child(count_chip(entry.resolved, theme::success()));
    }
    Some(node.into_any_element())
}

fn count_chip(count: usize, colour: Rgba) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .child(status_dot(colour, theme::transparent()))
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_muted())
                .child(SharedString::from(count.to_string())),
        )
        .into_any_element()
}
