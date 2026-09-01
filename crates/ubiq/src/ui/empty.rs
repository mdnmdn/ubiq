//! The pages a screen with nothing to show draws.
//!
//! Two scales, because a window's centre and a panel's body are not the same size of emptiness: the
//! centre explains itself and offers a way out, a panel says one line. What the footer holds is the
//! caller's, so a mode that is not built and a window with no project can share the layout without
//! sharing a claim about why they are empty.

use gpui::{AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, div, px};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::theme;
use crate::ui::kit::{ghost_button, mono};

pub fn empty_page(
    title: &str,
    note: &str,
    icon: IconName,
    footer: Option<AnyElement>,
) -> impl IntoElement {
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
        .children(footer)
}

/// The footer a rail mode with no screen behind it carries.
pub fn not_built() -> AnyElement {
    mono("not built yet", theme::text_faint())
        .text_size(px(11.))
        .into_any_element()
}

/// One muted line, centred: what a panel with nothing in it says.
pub fn empty_panel(note: &str) -> impl IntoElement {
    div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .px_3()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_size(px(12.5))
                .text_color(theme::text_faint())
                .child(SharedString::from(note.to_string())),
        )
}

/// The centre of a window holding no project: what the window is for, and the one way out of it.
///
/// The way out is the same platform folder chooser the picker's "Add a project…" row opens, so
/// there is one path into the catalogue rather than two. The chooser is followed by project
/// settings, which is where the name is set before the host is asked.
pub fn no_project(cx: &mut Context<AppState>) -> AnyElement {
    empty_page(
        "No project open",
        "Ubiq works in a project: a folder its agents run in, and the files they change. Open one \
         from the picker, or add a folder.",
        IconName::FolderOpen,
        Some(
            ghost_button(
                "empty-add-project",
                Some(IconName::Plus),
                "Add a project\u{2026}",
                cx.listener(|this, _, _, cx| this.choose_folder(None, cx)),
            )
            .into_any_element(),
        ),
    )
    .into_any_element()
}
