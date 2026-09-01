//! The file picker dialog.
//!
//! **It is drawn where it is asked for and painted over everything**, through `deferred` and
//! `anchored` like the kit's modal — so a picker raised from inside a dock panel covers the window
//! rather than being clipped to the panel that wanted it.
//!
//! The shape is the window's shape: square, filled, a coloured left edge. What is different from a
//! modal is that this dialog is *worked in* rather than answered — it holds a filter field, two
//! arrangements of the same set, a scroll, and a corner that resizes it — so it is bigger than a
//! modal, it remembers its size while it is up, and it says at the bottom how much has been chosen.
//!
//! **A row is one line, always.** A name or a folder that does not fit is elided and carries the
//! whole of itself as a tooltip; nothing here wraps, because a wrapped row would push the rest of
//! the list down and a list is scanned by its left edge.
//!
//! Whether an outside click dismisses it is the caller's, not this file's: a modal picker is
//! answered or cancelled, and one that is not modal goes away the moment attention leaves it.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, CursorStyle, InteractiveElement, IntoElement, KeyBinding, MouseButton,
    MouseDownEvent, MouseMoveEvent, ParentElement, SharedString, StatefulInteractiveElement,
    Styled, Window, anchored, deferred, div, point, px,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::file_picker::{FilePickerState, PickerCount, PickerKey, PickerRow, PickerView};
use crate::theme;
use crate::ui::eid;
use crate::ui::empty::empty_panel;
use crate::ui::kit::{
    check_box, elided, elided_with, file_row, filter_bar, ghost_button, icon_button, kind_icon,
    mono, primary_button, twisty, view_switch,
};

/// The key context the dialog is answered in, and the one the component library gives the field
/// inside it.
const CONTEXT: &str = "FilePicker";
const FIELD_CONTEXT: &str = "FilePicker > Input";

gpui::actions!(
    ubiq_picker,
    [
        PickerUp,
        PickerDown,
        PickerOut,
        PickerInto,
        PickerEnter,
        PickerConfirm,
        PickerDismiss
    ]
);

/// The keys the dialog answers to, bound twice each.
///
/// **The focus is in the filter field**, because typing a name is the first thing a picker is for,
/// and the component library's input binds `up`, `down`, `left`, `right`, `enter` and `escape` for
/// itself — at the deepest node in the tree, which is where a keymap's ties are broken. A binding
/// that only named the dialog would sit above the field and lose every one of them. So each key is
/// bound for the dialog *and* for the field inside it: the second predicate matches at the same
/// depth as the input's own and wins by being registered after it, which is why this is called from
/// `app::install_key_bindings` rather than anywhere earlier.
///
/// What the dialog does not answer, it hands back — [`AppState::press_picker_key`] answers false
/// and the handler propagates, so `left` and `right` are the field's caret keys again in the flat
/// list, where there is no tree to walk.
pub fn key_bindings() -> Vec<KeyBinding> {
    /// One key, for the dialog and for the field inside it.
    fn both<A: gpui::Action + Clone>(key: &str, action: A) -> [KeyBinding; 2] {
        [
            KeyBinding::new(key, action.clone(), Some(CONTEXT)),
            KeyBinding::new(key, action, Some(FIELD_CONTEXT)),
        ]
    }

    [
        both("up", PickerUp),
        both("down", PickerDown),
        both("left", PickerOut),
        both("right", PickerInto),
        both("enter", PickerEnter),
        // What the platform calls confirm: cmd on macOS, ctrl everywhere else.
        both("secondary-enter", PickerConfirm),
        both("escape", PickerDismiss),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// One key, answered by the picker if it means anything there and handed back if it does not.
fn answer(this: &mut AppState, key: PickerKey, cx: &mut Context<AppState>) {
    if !this.press_picker_key(key, cx) {
        // Nothing here wanted it: the field is welcome to it.
        cx.propagate();
    }
}

pub fn render(
    app: &AppState,
    picker: &FilePickerState,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let viewport = window.viewport_size();
    let modal = picker.request.modal;
    let tree = picker.view == PickerView::Tree;
    let multiple = picker.request.count == PickerCount::Multiple;

    let rows: Vec<AnyElement> = picker
        .rows()
        .into_iter()
        .map(|row| line(row, tree, multiple, cx))
        .collect();
    let nothing = rows.is_empty();

    let panel = div()
        .id("file-picker-panel")
        .key_context(CONTEXT)
        .on_action(cx.listener(|this, _: &PickerUp, _, cx| answer(this, PickerKey::Up, cx)))
        .on_action(cx.listener(|this, _: &PickerDown, _, cx| answer(this, PickerKey::Down, cx)))
        .on_action(cx.listener(|this, _: &PickerOut, _, cx| answer(this, PickerKey::Left, cx)))
        .on_action(cx.listener(|this, _: &PickerInto, _, cx| answer(this, PickerKey::Right, cx)))
        .on_action(cx.listener(|this, _: &PickerEnter, _, cx| answer(this, PickerKey::Enter, cx)))
        .on_action(
            cx.listener(|this, _: &PickerConfirm, _, cx| answer(this, PickerKey::Confirm, cx)),
        )
        .on_action(
            cx.listener(|this, _: &PickerDismiss, _, cx| answer(this, PickerKey::Dismiss, cx)),
        )
        .relative()
        .w(px(picker.width))
        .h(px(picker.height))
        .max_w(viewport.width - px(32.))
        .max_h(viewport.height - px(32.))
        .flex()
        .flex_col()
        .bg(theme::surface_raised())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::accent())
        .shadow_lg()
        .child(header(picker, cx))
        .child(field(app, picker))
        .child(
            div()
                .id("file-picker-rows")
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .track_scroll(&app.picker_scroll)
                .children(nothing.then(|| empty_panel("Nothing matches")))
                .children(rows),
        )
        .child(footer(picker, cx))
        .child(grip(cx))
        // A picker that does not hold the window goes away when attention leaves it. A modal one
        // stays up until it is answered or cancelled, which is what modal means.
        .when(!modal, |this| {
            this.on_mouse_down_out(cx.listener(|this, _, _, cx| this.cancel_file_picker(cx)))
        });

    // The layer is full-window in both cases: it is what the resize drag is tracked on, because a
    // pointer that outran the corner would otherwise leave the dialog and strand the drag.
    let mut layer = div()
        .id("file-picker")
        .w(viewport.width)
        .h(viewport.height)
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
            if event.dragging() {
                let at = (f32::from(event.position.x), f32::from(event.position.y));
                this.drag_picker_resize(at, window, cx);
            }
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| this.end_picker_resize(cx)),
        );

    if modal {
        // The scrim occludes, so nothing behind a modal picker can be clicked while it is up.
        layer = layer.bg(theme::scrim()).occlude();
    }

    deferred(
        anchored()
            .position(point(px(0.), px(0.)))
            .child(layer.child(panel)),
    )
    // Above the kit's dropdowns, which sit at 1, for the same reason a modal is.
    .priority(2)
    .into_any_element()
}

/// What is being chosen, in what shape, and which arrangement it is in.
///
/// The two chips are the request made visible: a dialog that takes several files and one that takes
/// exactly one look alike until the footer, and the header is where a user finds out which they are
/// in.
fn header(picker: &FilePickerState, cx: &mut Context<AppState>) -> AnyElement {
    // let count = match picker.request.count {
    // PickerCount::Single => "single",
    // PickerCount::Multiple => "multi",
    // };

    div()
        .h(px(44.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex_shrink(1.0)
                .min_w(px(0.))
                .text_size(px(15.))
                .text_color(theme::text())
                .truncate()
                .child(SharedString::from(picker.request.title.clone())),
        )
        // .child(chip(picker.request.kind.label()))
        // .child(chip(count))
        .child(div().flex_1().min_w(px(0.)))
        .child(view_switch(
            "file-picker-tree",
            "file-picker-list",
            picker.view == PickerView::Tree,
            cx.listener(|this, _, _, cx| this.set_picker_view(PickerView::Tree, cx)),
            cx.listener(|this, _, _, cx| this.set_picker_view(PickerView::List, cx)),
        ))
        .child(icon_button(
            "file-picker-close",
            IconName::Close,
            false,
            cx.listener(|this, _, _, cx| this.cancel_file_picker(cx)),
        ))
        .into_any_element()
}

/// A mono chip saying one word about the request. Not a control: nothing about the ask changes
/// while the dialog is up.
fn chip(label: &str) -> impl IntoElement {
    mono(SharedString::from(label.to_string()), theme::text_muted())
        .text_size(px(11.))
        .flex_none()
        .px_1p5()
        .py(px(2.))
        .border_1()
        .border_color(theme::border())
}

/// The filter field, the prefilter it is on top of, and the name of the arrangement below it.
///
/// One field over both views on purpose: what was typed survives the toggle, because a user who
/// cannot find something in the tree switches to the list to look for the same thing.
fn field(app: &AppState, picker: &FilePickerState) -> impl IntoElement {
    filter_bar(
        Input::new(&app.picker_filter).appearance(false),
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .children(picker.request.pattern.clone().map(|pattern| {
                mono(pattern, theme::text_faint())
                    .text_size(px(10.5))
                    .flex_none()
                    .px_1()
                    .bg(theme::surface_raised())
            }))
            .child(
                mono(picker.view.label(), theme::text_faint())
                    .text_size(px(10.5))
                    .flex_none()
                    .px_1()
                    .bg(theme::surface_raised()),
            ),
    )
}

/// One row: what it is, what it is called, and what it says at its far end.
///
/// The tick box is drawn only where several answers are possible. A single pick says which one it
/// is with the row's own accent edge, because a lone tick box invites a second tick the dialog
/// would then have to take away.
///
/// The twisty is its own click target and stops the click there, so a folder that *can* be picked
/// is opened by its chevron and chosen by its row — the one case where the two mean different
/// things.
fn line(row: PickerRow, tree: bool, multiple: bool, cx: &mut Context<AppState>) -> AnyElement {
    let path = row.path.clone();
    let ticks = row.pickable && multiple;
    let selected = row.selected;

    let mut line = file_row(
        eid("picker-row", &row.path),
        row.depth,
        selected,
        row.on_cursor,
    );

    if tree && row.is_dir {
        let folder = row.path.clone();
        line = line.child(twisty(
            eid("picker-twisty", &row.path),
            row.expanded,
            cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_picker_folder(folder.clone(), cx);
            }),
        ));
    }

    if ticks {
        let ticked = row.path.clone();
        line = line.child(check_box(
            eid("picker-tick", &row.path),
            selected,
            cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.click_picker_row(ticked.clone(), cx);
            }),
        ));
    }

    line = line
        .child(kind_icon(row.is_dir, theme::text_faint()))
        .child(elided_with(
            eid("picker-name", &row.path),
            row.name.clone(),
            // The whole path, not the name again: a name that fits still leaves "which one is
            // this" open, and in the flat list two folders can agree on one.
            match row.path.is_empty() {
                true => row.name.clone(),
                false => row.path.clone(),
            },
            match row.pickable {
                true => theme::text(),
                false => theme::text_muted(),
            },
            13.0,
        ));

    if !row.trailing.is_empty() {
        line = line.child(
            div()
                .flex()
                .flex_none()
                .max_w(px(180.))
                .font_family(theme::MONO_FONT)
                .child(elided(
                    eid("picker-trailing", &row.path),
                    row.trailing.clone(),
                    theme::text_faint(),
                    11.5,
                )),
        );
    }

    line.on_click(cx.listener(move |this, _, _, cx| this.click_picker_row(path.clone(), cx)))
        .into_any_element()
}

/// How much has been chosen, and the two ways out.
///
/// The confirming button is absent when a click on a row is already the answer: a dialog that
/// closes on the click has nothing left for a button to do, and one drawn there would look like a
/// step the user is skipping.
fn footer(picker: &FilePickerState, cx: &mut Context<AppState>) -> AnyElement {
    let ready = picker.can_commit();
    let label = picker.confirm_label();

    div()
        .px_3()
        .py_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(theme::pane_bg())
        .border_t_1()
        .border_color(theme::border())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .font_family(theme::MONO_FONT)
                .child(elided(
                    "file-picker-tally",
                    picker.tally(),
                    theme::text_muted(),
                    11.5,
                )),
        )
        .child(hint(picker))
        .child(ghost_button(
            "file-picker-cancel",
            None,
            "Cancel",
            cx.listener(|this, _, _, cx| this.cancel_file_picker(cx)),
        ))
        .children((!picker.request.commits_on_click()).then(|| {
            primary_button(
                "file-picker-confirm",
                None,
                label,
                cx.listener(|this, _, _, cx| this.commit_file_picker(cx)),
            )
            // Nothing chosen is nothing to add. The button stays where it is and drains, rather
            // than disappearing and moving Cancel under the pointer.
            .when(!ready, |button| {
                button.bg(theme::fade(theme::accent(), 0.35))
            })
        }))
        .into_any_element()
}

/// What the keyboard can do here, said once where it is needed.
///
/// A dialog that answers arrow keys and says nothing about it is a dialog nobody uses them in. The
/// wording changes with what was asked for, because half of it would be a lie otherwise: there is
/// no "add" in a single pick, and no folders to walk in the flat list.
fn hint(picker: &FilePickerState) -> impl IntoElement {
    let confirm = match cfg!(target_os = "macos") {
        true => "\u{2318}\u{23ce} add",
        false => "ctrl-\u{23ce} add",
    };

    let mut parts = vec!["\u{2191}\u{2193} move"];
    if picker.view == PickerView::Tree {
        parts.push("\u{2190}\u{2192} folders");
    }
    match picker.request.count {
        PickerCount::Single => parts.push("\u{23ce} select"),
        PickerCount::Multiple => {
            parts.push("\u{23ce} tick");
            parts.push(confirm);
        }
    }
    parts.push("esc close");

    mono(parts.join("  \u{00b7}  "), theme::text_faint())
        .flex_none()
        .text_size(px(10.5))
}

/// The corner the dialog is resized by.
///
/// The dialog is centred, so a drag of the corner moves both of its edges: the grip follows the
/// pointer and the far side moves the same distance the other way. That arithmetic is
/// [`FilePickerState::drag_to`]'s, not this file's.
fn grip(cx: &mut Context<AppState>) -> impl IntoElement {
    div()
        .id("file-picker-grip")
        .absolute()
        .bottom(px(0.))
        .right(px(0.))
        .size(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .cursor(CursorStyle::ResizeUpLeftDownRight)
        .child(
            Icon::new(IconName::ResizeCorner)
                .with_size(Size::XSmall)
                .text_color(theme::text_faint()),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                let at = (f32::from(event.position.x), f32::from(event.position.y));
                this.start_picker_resize(at, cx);
            }),
        )
}
