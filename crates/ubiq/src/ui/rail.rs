//! The activity rail: the app's destinations, grouped, with exactly one active.

use gpui::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::RailMode;
use crate::theme;
use crate::ui::kit::section_label;

/// The rail's glyph for a mode. Icons come from the component library's bundle; Ubiq ships none.
pub fn mode_icon(mode: RailMode) -> IconName {
    match mode {
        RailMode::Control => IconName::LayoutDashboard,
        RailMode::Ide => IconName::SquareTerminal,
        RailMode::Agents => IconName::Asterisk,
        RailMode::Kb => IconName::BookOpen,
        RailMode::Tasks => IconName::CircleCheck,
        RailMode::Sink => IconName::Palette,
    }
}

/// The mark: the project's colour and a U, sitting in the titlebar row above the rail so the two
/// read as one column.
pub fn mark(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let project = app.project_tint(cx);
    div()
        .w(px(theme::RAIL_WIDTH))
        .h(px(theme::TITLEBAR_HEIGHT))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .bg(theme::pane_bg())
        .border_r_1()
        .border_color(theme::border())
        .child(
            div()
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .bg(project)
                .text_color(theme::on_accent())
                .text_size(px(15.))
                .child("U"),
        )
}

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let active = app.workbench.rail_mode;

    let mut groups = Vec::new();
    for (label, modes) in RailMode::groups() {
        let mut items = Vec::new();
        for mode in *modes {
            items.push(rail_item(*mode, *mode == active, cx));
        }
        groups.push(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .pb_3()
                .child(div().pt_3().pb_1().child(section_label(label)))
                .children(items),
        );
    }

    div()
        .w(px(theme::RAIL_WIDTH))
        .flex()
        .flex_none()
        .flex_col()
        .items_center()
        .bg(theme::pane_bg())
        .border_r_1()
        .border_color(theme::border())
        .children(groups)
}

fn rail_item(mode: RailMode, active: bool, cx: &mut Context<AppState>) -> AnyElement {
    let (fg, bg) = if active {
        (theme::accent(), theme::accent_soft())
    } else {
        (theme::text_muted(), theme::pane_bg())
    };

    div()
        .id(ElementId::Name(
            format!("rail-{}", mode.label().to_lowercase()).into(),
        ))
        .w(px(56.))
        .py_2()
        .flex()
        .flex_none()
        .flex_col()
        .items_center()
        .gap_1()
        .bg(bg)
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(
            Icon::new(mode_icon(mode))
                .with_size(Size::Medium)
                .text_color(fg),
        )
        .child(
            div()
                .text_size(px(10.5))
                .text_color(fg)
                .child(SharedString::from(mode.label())),
        )
        .on_click(cx.listener(move |this, _, _, cx| this.set_rail_mode(mode, cx)))
        .into_any_element()
}
