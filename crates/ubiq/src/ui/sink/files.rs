//! The file picker page: the dialog raised in each of the shapes a screen can ask for it.
//!
//! **The controls are the request, one row each.** What it picks, how many, when a single pick is
//! final, whether it holds the window, which arrangement it opens in, which folder it is rooted at
//! and what it is prefiltered by — the seven fields of a
//! [`crate::state::file_picker::PickerRequest`], drawn as the pill rows the rest of the sink uses.
//! Setting them and raising the dialog is how a caller's ask is looked at before a screen depends
//! on it.
//!
//! **What comes back is printed under the button.** A picker whose answer went nowhere would be a
//! picture of a picker: the readout is the sink's stand-in for the screen that will one day receive
//! the paths, and it says the difference between a dialog that was cancelled and one that answered
//! with nothing.
//!
//! The dialog itself is not drawn here. It belongs to the window — one may be up at a time — so
//! [`crate::ui::file_picker`] draws it and [`super::render`] paints it over whichever page is on.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px, relative,
};

use crate::app::AppState;
use crate::state::file_picker::{Commit, PickKind, PickerCount, PickerView};
use crate::state::sink::{PICKER_PATTERNS, PICKER_ROOTS};
use crate::theme;
use crate::ui::kit::{choice_pill, elided, mono, primary_button};
use crate::ui::sink::style::{group, labelled, row};

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .id("sink-files")
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::app_bg())
        .overflow_y_scroll()
        .child(ask(app, cx))
        .child(answer(app))
        .into_any_element()
}

/// Every field of the request, and the button that raises a dialog out of them.
fn ask(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let demo = &app.sink.picker;

    let kinds = labelled(
        "kind",
        pills(
            "sink-pick-kind",
            &[("files", PickKind::Files), ("folders", PickKind::Folders)],
            demo.kind,
            cx,
            |this, value, cx| this.set_sink_pick_kind(value, cx),
        ),
    );

    let counts = labelled(
        "count",
        pills(
            "sink-pick-count",
            &[
                ("one", PickerCount::Single),
                ("several", PickerCount::Multiple),
            ],
            demo.count,
            cx,
            |this, value, cx| this.set_sink_pick_count(value, cx),
        ),
    );

    let commits = labelled(
        "commit",
        pills(
            "sink-pick-commit",
            &[
                ("on the click", Commit::OnClick),
                ("on the button", Commit::OnButton),
            ],
            demo.commit,
            cx,
            |this, value, cx| this.set_sink_pick_commit(value, cx),
        ),
    );

    let modality = labelled(
        "modal",
        pills(
            "sink-pick-modal",
            &[("holds the window", true), ("click outside to go", false)],
            demo.modal,
            cx,
            |this, value, cx| this.set_sink_pick_modal(value, cx),
        ),
    );

    let views = labelled(
        "view",
        pills(
            "sink-pick-view",
            &[("tree", PickerView::Tree), ("list", PickerView::List)],
            demo.view,
            cx,
            |this, value, cx| this.set_sink_pick_view(value, cx),
        ),
    );

    let roots = labelled(
        "root",
        pills(
            "sink-pick-root",
            &PICKER_ROOTS
                .iter()
                .enumerate()
                .map(|(index, (label, _))| (*label, index))
                .collect::<Vec<_>>(),
            demo.root,
            cx,
            |this, value, cx| this.set_sink_pick_root(value, cx),
        ),
    );

    let patterns = labelled(
        "pattern",
        pills(
            "sink-pick-pattern",
            &PICKER_PATTERNS
                .iter()
                .enumerate()
                .map(|(index, (label, _))| (*label, index))
                .collect::<Vec<_>>(),
            demo.pattern,
            cx,
            |this, value, cx| this.set_sink_pick_pattern(value, cx),
        ),
    );

    group(
        "The ask",
        "Every field of a picker request, and the dialog it adds up to. The tree it opens over is \
         a fixture: the sink has no project behind it.",
        vec![
            row(vec![kinds, counts, commits]),
            row(vec![modality, views]),
            row(vec![roots, patterns]),
            div()
                .flex()
                .flex_none()
                .child(primary_button(
                    "sink-pick-raise",
                    None,
                    "Raise the picker",
                    cx.listener(|this, _, window, cx| this.raise_sink_picker(window, cx)),
                ))
                .into_any_element(),
        ],
    )
}

/// What the last dialog handed back.
///
/// Three answers rather than two: nothing asked yet, dismissed, and a list — which may itself be
/// empty, and an empty list is a real answer rather than a dismissal.
fn answer(app: &AppState) -> AnyElement {
    let demo = &app.sink.picker;

    let body: Vec<AnyElement> = match (&demo.result, demo.dismissed) {
        (_, true) => vec![note("Dismissed. Nothing came back.")],
        (None, _) => vec![note("Nothing asked yet.")],
        (Some(paths), _) if paths.is_empty() => vec![note("Answered with nothing.")],
        (Some(paths), _) => paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                div()
                    .h(px(24.))
                    .w(relative(1.))
                    .max_w(px(520.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .bg(theme::surface())
                    .px_2()
                    .border_l(px(theme::ACCENT_EDGE))
                    .border_color(theme::accent())
                    .child(elided(
                        ("sink-pick-result", index),
                        SharedString::from(path.clone()),
                        theme::text(),
                        12.0,
                    ))
                    .into_any_element()
            })
            .collect(),
    };

    group(
        "What came back",
        "The paths the picker handed over, in the order they were chosen. Project-relative, as \
         every path the interface holds is.",
        vec![
            div()
                .flex()
                .flex_none()
                .flex_col()
                .gap_1()
                .children(body)
                .into_any_element(),
        ],
    )
}

fn note(text: &str) -> AnyElement {
    mono(SharedString::from(text.to_string()), theme::text_faint())
        .text_size(px(11.5))
        .into_any_element()
}

/// One row of pills over one field of the request: the values, and which of them is set.
fn pills<T: Copy + PartialEq + 'static>(
    id: &'static str,
    values: &[(&str, T)],
    current: T,
    cx: &mut Context<AppState>,
    set: fn(&mut AppState, T, &mut Context<AppState>),
) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .children(values.iter().map(|(label, value)| {
            let value = *value;
            choice_pill(
                (id, label_key(label)),
                SharedString::from(label.to_string()),
                value == current,
                cx.listener(move |this, _, _, cx| set(this, value, cx)),
            )
        }))
        .into_any_element()
}

/// A stable number for a pill's label, so two rows of pills cannot collide on an element id.
fn label_key(label: &str) -> u64 {
    label.bytes().fold(0u64, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte as u64)
    })
}
