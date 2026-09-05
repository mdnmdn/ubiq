//! The ⌘K navigator, drawn under the titlebar's command field.
//!
//! Painted through `deferred` and `anchored` — the device the kit's dropdown uses — so the list
//! hangs off the field it belongs to rather than being clipped to the titlebar's row. The key
//! context and every handler go on the **field**, not on this panel: the keyboard is in the input
//! inside the field, and a deferred panel is nowhere on the focus path.
//!
//! **A row is one line, always**, elided with the whole of itself as a tooltip — the same rule the
//! file picker's rows keep, for the same reason: a wrapped row pushes the list under it down.

use gpui::{
    Context, Div, InteractiveElement, IntoElement, KeyBinding, ParentElement,
    StatefulInteractiveElement as _, Styled, anchored, deferred, div, point, px,
};

use crate::app::AppState;
use crate::state::navigator::{Group, NavRow, NavigatorState};
use crate::theme;
use crate::ui::kit::{ROW_FONT, elided, file_row, section_label};

/// The context the navigator is answered in, and the one the component library gives the field
/// inside it. The field wears both: `Navigator` sits on the field's own div, so the input it holds
/// is a descendant and the deeper predicate matches too.
const CONTEXT: &str = "Navigator";
const FIELD_CONTEXT: &str = "Navigator > Input";

gpui::actions!(
    ubiq_navigator,
    [NavigatorUp, NavigatorDown, NavigatorEnter, NavigatorDismiss]
);

/// The keys the navigator answers to, bound twice each.
///
/// Same device — and same reason — as [`crate::ui::file_picker::key_bindings`]: the focus sits in a
/// field the component library has already bound `up`, `down`, `enter` and `escape` for, at the
/// deepest node in the tree. A binding that only named the dialog would sit above the field and
/// lose every one of them, so each key is bound for the navigator *and* for the field inside it.
pub fn key_bindings() -> Vec<KeyBinding> {
    /// One key, for the navigator and for the field inside it.
    fn both<A: gpui::Action + Clone>(key: &str, action: A) -> [KeyBinding; 2] {
        [
            KeyBinding::new(key, action.clone(), Some(CONTEXT)),
            KeyBinding::new(key, action, Some(FIELD_CONTEXT)),
        ]
    }

    [
        both("up", NavigatorUp),
        both("down", NavigatorDown),
        both("enter", NavigatorEnter),
        both("escape", NavigatorDismiss),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Put the navigator on the titlebar's field, when one is up. A no-op otherwise: the field is
/// handed back untouched, and its Enter is the project search it has always been.
pub fn attach(field: Div, app: &AppState, cx: &mut Context<AppState>) -> Div {
    let Some(nav) = &app.navigator else {
        return field;
    };
    field
        .key_context(CONTEXT)
        .on_action(cx.listener(|this, _: &NavigatorUp, _, cx| this.move_navigator(false, cx)))
        .on_action(cx.listener(|this, _: &NavigatorDown, _, cx| this.move_navigator(true, cx)))
        .on_action(cx.listener(|this, _: &NavigatorEnter, window, cx| {
            let at = this.navigator.as_ref().map_or(0, |nav| nav.cursor);
            this.press_navigator(at, window, cx);
        }))
        // Escape takes the list away and leaves the text alone: closing the list is not undoing
        // the typing.
        .on_action(cx.listener(|this, _: &NavigatorDismiss, _, cx| this.close_navigator(cx)))
        .child(panel(app, nav, cx))
}

/// The list itself, dropped the height of the titlebar so it sits under the field rather than over
/// it — `anchored` with no position anchors at the element it is a child of.
fn panel(app: &AppState, nav: &NavigatorState, cx: &mut Context<AppState>) -> impl IntoElement {
    let found = app.navigator_rows(cx);
    let at = nav.cursor.min(found.len().saturating_sub(1));

    let mut children: Vec<gpui::AnyElement> = Vec::new();
    let mut group: Option<Group> = None;
    for (index, row) in found.iter().enumerate() {
        if group != Some(row.group) {
            group = Some(row.group);
            children.push(
                div()
                    .px_2()
                    .pt_1p5()
                    .pb_0p5()
                    .flex_none()
                    .child(section_label(row.group.label()))
                    .into_any_element(),
            );
        }
        children.push(line(index, row, index == at, cx));
    }
    if children.is_empty() {
        children.push(
            div()
                .h(px(28.))
                .px_2()
                .flex()
                .items_center()
                .text_size(px(12.5))
                .text_color(theme::text_faint())
                .child("Nowhere to go")
                .into_any_element(),
        );
    }

    deferred(
        anchored()
            .offset(point(px(0.), px(theme::TITLEBAR_HEIGHT)))
            .snap_to_window_with_margin(px(8.))
            .child(
                div()
                    .id("navigator-panel")
                    .w(px(420.))
                    .max_h(px(420.))
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .bg(theme::surface_raised())
                    .border_l(px(theme::ACCENT_EDGE))
                    .border_color(theme::accent())
                    .shadow_lg()
                    .children(children),
            ),
    )
    // Above the kit's dropdowns, which sit at 1: the navigator is raised over the whole chrome.
    .priority(2)
}

/// One row: what it is called, and what it says at its far end.
///
/// A row that names no place — a link to a project this catalogue does not hold — draws faint and
/// takes no click: there is nowhere for it to go.
fn line(
    index: usize,
    row: &NavRow,
    on_cursor: bool,
    cx: &mut Context<AppState>,
) -> gpui::AnyElement {
    let colour = match (row.dest.is_some(), row.adrift) {
        (false, _) => theme::text_faint(),
        // A bookmark that has lost its line says so rather than pretending to still hold it.
        (true, true) => theme::warning(),
        (true, false) => theme::text(),
    };
    let whole = match row.detail.is_empty() {
        true => row.label.clone(),
        false => format!("{} — {}", row.label, row.detail),
    };

    let mut line = file_row(
        ("navigator-row", index),
        0,
        false,
        on_cursor,
        true,
        ROW_FONT,
    )
    .child(elided(
        ("navigator-label", index),
        row.label.clone(),
        colour,
        12.5,
    ))
    .child(
        div()
            .flex()
            .flex_none()
            .max_w(px(180.))
            .font_family(theme::MONO_FONT)
            .child(elided(
                ("navigator-detail", index),
                row.detail.clone(),
                theme::text_faint(),
                11.,
            )),
    )
    .tooltip(move |window, cx| {
        gpui_component::tooltip::Tooltip::new(whole.clone()).build(window, cx)
    });

    if row.dest.is_some() {
        line = line.on_click(
            cx.listener(move |this, _, window, cx| this.press_navigator(index, window, cx)),
        );
    }
    line.into_any_element()
}
