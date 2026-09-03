//! The activity rail: the app's destinations, grouped, with exactly one active.

use std::sync::Arc;

use gpui::{
    AnyElement, Context, ElementId, Image, ImageFormat, ImageSource, InteractiveElement,
    IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, div, img, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::RailMode;
use crate::theme;
use crate::ui::kit::section_label;

/// The mark's two files: the white logo reads on a dark swatch, the blue on a light one. They are
/// the only assets Ubiq ships, so they are baked in next to the code that draws them.
const LOGO_WHITE: &[u8] = include_bytes!("../../../../assets/logo-white.png");
const LOGO_BLUE: &[u8] = include_bytes!("../../../../assets/logo-blue.png");

/// The rail's glyph for a mode. Icons come from the component library's bundle; Ubiq ships none.
pub fn mode_icon(mode: RailMode) -> IconName {
    match mode {
        RailMode::Control => IconName::LayoutDashboard,
        RailMode::Ide => IconName::SquareTerminal,
        RailMode::Git => IconName::GalleryVerticalEnd,
        RailMode::Agents => IconName::Asterisk,
        RailMode::Orchestration => IconName::Network,
        RailMode::Kb => IconName::BookOpen,
        RailMode::Tasks => IconName::CircleCheck,
        RailMode::Sink => IconName::Palette,
    }
}

/// The mark: the project's colour and the logo, sitting in the titlebar row above the rail so the
/// two read as one column. The logo is the pale file on a dark swatch and the blue one on a light
/// swatch, so it stays legible on whatever the project is tinted.
pub fn mark(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let (tint, white) = match app.project_snapshot(cx) {
        Some(project) => {
            let tint = theme::project_tint(
                project.record.temporary,
                project.record.colour,
                project.record.custom_colour,
            );
            (tint, theme::mark_dark(tint))
        }
        None => (theme::border(), theme::mark_dark(theme::border())),
    };
    let logo = Arc::new(Image::from_bytes(
        ImageFormat::Png,
        (if white { LOGO_WHITE } else { LOGO_BLUE }).to_vec(),
    ));
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
                .bg(tint)
                .child(img(ImageSource::Image(logo)).size_full()),
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
