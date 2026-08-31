//! The bottom strip: where the working tree, the caret and the harness selection are reported.

use gpui::{Context, IntoElement, ParentElement, SharedString, Styled, div, px};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::theme;
use crate::ui::kit::mono;

/// Where this run writes everything down, when that is not `~/.config/ubiq`.
fn config_root(app: &AppState) -> Option<impl IntoElement> {
    if app.workbench.config_root_is_default {
        return None;
    }
    let root = app.workbench.config_root.clone()?;
    let shown = match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => root.replace(&home, "~"),
        _ => root,
    };

    Some(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Icon::new(IconName::Inspector)
                    .with_size(Size::XSmall)
                    .text_color(theme::warning()),
            )
            .child(mono(shown, theme::warning())),
    )
}

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let wb = &app.workbench;
    let (line, column) = app.cursor_line_column(cx);

    let tree = format!(
        "{} modified \u{b7} {} untracked \u{b7} {} conflict",
        wb.modified, wb.untracked, wb.conflicts
    );
    let language = app
        .editor
        .active_file()
        .map(|f| f.language.label())
        .unwrap_or("Plain Text");

    div()
        .h(px(theme::STATUS_BAR_HEIGHT))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_4()
        .bg(theme::pane_bg())
        .border_t_1()
        .border_color(theme::border())
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Icon::new(IconName::Network)
                        .with_size(Size::XSmall)
                        .text_color(theme::text_faint()),
                )
                .child(mono(
                    format!(
                        "{} \u{2191}{} \u{2193}{}",
                        wb.branch_name(),
                        wb.ahead,
                        wb.behind
                    ),
                    theme::text_muted(),
                )),
        )
        .child(mono(tree, theme::text_muted()))
        .child(div().flex_1().min_w(px(0.)))
        // A config root you cannot see is a foot-gun, so a run pointed anywhere but the usual
        // place says so.
        .children(config_root(app))
        .child(mono(
            format!("Ln {line}, Col {column}"),
            theme::text_muted(),
        ))
        .child(mono(
            format!("{language} \u{b7} UTF-8 \u{b7} LF"),
            theme::text_muted(),
        ))
        .child(mono(
            SharedString::from(format!(
                "{} \u{b7} {}",
                app.chat.harness_label(),
                app.chat.mode_label()
            )),
            theme::text_muted(),
        ))
}
