//! The chat panel.
//!
//! It is the one panel that stays on screen in every rail mode. The transport contract has no chat
//! family yet, so everything it shows is UI-local state seeded from `state::sample`.

pub mod composer;
pub mod sidebar;
pub mod transcript;

use gpui::{Context, IntoElement, ParentElement, Styled, div, px};

use crate::app::AppState;
use crate::theme;
use crate::ui::kit::panel;

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let mut root = panel()
        .border_l_1()
        .border_color(theme::border())
        .child(sidebar::header(app, cx));

    if !app.chat.collapsed {
        root = root.child(sidebar::chat_list(app, cx));
    }

    root.child(sidebar::status_strip(app, cx))
        .child(transcript::render(app, cx))
        .child(
            div()
                .flex_none()
                .min_h(px(0.))
                .child(composer::render(app, cx)),
        )
}
