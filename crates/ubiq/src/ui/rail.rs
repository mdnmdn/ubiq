//! The activity rail: the app's destinations, grouped, with exactly one active.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;

use gpui::{
    AnyElement, Context, ElementId, Image, ImageFormat, ImageSource, InteractiveElement,
    IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div, img,
    px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::{RailMode, WindowRegistry};
use crate::theme;
use crate::ui::kit::section_label;

/// The mark's two files: the white logo reads on a dark swatch, the blue on a light one. They are
/// the only assets Ubiq ships, so they are baked in next to the code that draws them.
const LOGO_WHITE: &[u8] = include_bytes!("../../../../assets/logo-white.png");
const LOGO_BLUE: &[u8] = include_bytes!("../../../../assets/logo-blue.png");

/// What the rail spends per mode and per group heading. Fixed rather than measured: the badges
/// under the modes have to know, while the rail is being built, how much room is left over, and
/// nothing is measured until it is painted.
const ITEM_HEIGHT: f32 = 52.0;
/// The rail's own width less its border — what everything inside it is exactly as wide as. Fixed
/// rather than `w_full`, because a mode's label is wider than the rail and a flex child's
/// automatic minimum size would let that label push its row wider than the rows beside it.
const ITEM_WIDTH: f32 = theme::RAIL_WIDTH - 1.0;
const GROUP_HEIGHT: f32 = 42.0;
/// One project badge: the rail's width less its own border, which is what the selected badge
/// fills edge to edge.
const BADGE_HEIGHT: f32 = theme::RAIL_WIDTH - 1.0;
/// What an unselected badge keeps clear of the rail's edges, and how thick its ring is.
const BADGE_MARGIN: f32 = 3.0;

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
        .bg(tint)
        .border_r_1()
        // The bottom edge is the titlebar's own line carried across, so the swatch ends where the
        // project chip beside it ends.
        .border_b_1()
        .border_color(theme::border())
        .child(img(ImageSource::Image(logo)).size(px(theme::TITLEBAR_HEIGHT - 10.)))
}

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> impl IntoElement {
    let active = app.workbench.rail_mode;

    let mut groups = Vec::new();
    let mut spent = 0.0;
    for (label, modes) in RailMode::groups() {
        let mut items = Vec::new();
        for mode in *modes {
            if !app.mode_enabled(*mode, cx) {
                continue;
            }
            items.push(rail_item(*mode, *mode == active, cx));
        }
        // A group whose every mode is hidden takes its heading with it.
        if items.is_empty() {
            continue;
        }
        spent += GROUP_HEIGHT + ITEM_HEIGHT * items.len() as f32;
        groups.push(
            div()
                .w(px(ITEM_WIDTH))
                .flex()
                .flex_col()
                .items_center()
                .pb_3()
                .child(
                    div()
                        .h(px(GROUP_HEIGHT - 12.))
                        .pt_3()
                        .child(section_label(label)),
                )
                .children(items),
        );
    }

    // The modes come first, always: the badges take whatever whole ones are left over, and none
    // when the window is too short for even one.
    let room = f32::from(window.viewport_size().height)
        - theme::TITLEBAR_HEIGHT
        - theme::STATUS_BAR_HEIGHT;
    // The last badge keeps a pixel off the bottom edge as well.
    let fits = ((room - spent - 1.) / BADGE_HEIGHT).floor().max(0.) as usize;

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
        .child(div().flex_1().min_h(px(0.)))
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .pb(px(1.))
                .children(project_badges(app, fits, cx)),
        )
}

/// The projects this window holds, at the bottom of the rail.
///
/// A reminder rather than a picker: the badge is the project's colour and its initial, the name is
/// the tooltip, and the one the window is pointed at wears a ring inside its own edge. Off by
/// default is not offered — the switch lives in appearance settings, and off means this returns
/// nothing at all.
///
/// **The order never moves.** Badges are drawn in the order the window holds them, whatever the
/// rail has room for; a shortage drops the least recently opened, so the ones that remain stay
/// where the user last saw them.
fn project_badges(app: &AppState, fits: usize, cx: &mut Context<AppState>) -> Vec<AnyElement> {
    if !app.workbench.settings.ui.rail_projects || fits == 0 {
        return Vec::new();
    }
    // One project is nothing to pick between, so the badge would only repeat the mark above it.
    let Some(slot) = app.window_slot(cx).filter(|slot| slot.projects.len() > 1) else {
        return Vec::new();
    };
    let registry = WindowRegistry::read(cx);
    let active = slot.active_project();
    // Which ones to keep is a question of recency; where they go is not. The active project is the
    // most recent by definition, so it is the badge that survives when there is room for only one.
    let mut keep: Vec<_> = slot.projects.clone();
    keep.sort_by_key(|id| {
        (
            Some(*id) == active,
            registry.project(*id).map(|p| p.record.last_opened_at),
        )
    });
    keep.drain(..slot.projects.len().saturating_sub(fits));

    slot.projects
        .iter()
        .filter(|id| keep.contains(id))
        .filter_map(|id| registry.project(*id))
        .map(|p| {
            (
                p.record.id,
                p.record.name.clone(),
                theme::project_tint(p.record.temporary, p.record.colour, p.record.custom_colour),
            )
        })
        .map(|(id, name, tint)| {
            let initial = name
                .chars()
                .next()
                .unwrap_or('?')
                .to_uppercase()
                .to_string();
            let tooltip = SharedString::from(name);
            let selected = Some(id) == active;
            div()
                .id(ElementId::Name(format!("rail-project-{id}").into()))
                // The window's own project is the full square, edge to edge; the rest are the
                // same square drawn as a thick ring, inset so the two never read as one block.
                .m(px(if selected { 0. } else { BADGE_MARGIN }))
                .size(px(if selected {
                    BADGE_HEIGHT
                } else {
                    BADGE_HEIGHT - BADGE_MARGIN * 2.
                }))
                .when(!selected, |this| {
                    this.border(px(BADGE_MARGIN)).border_color(tint)
                })
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .when(selected, |this| this.bg(tint))
                .text_color(if selected { theme::on_accent() } else { tint })
                .text_size(px(15.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .cursor_pointer()
                .child(initial)
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
                })
                .on_click(cx.listener(move |this, _, _, cx| this.activate_project(id, cx)))
                .into_any_element()
        })
        .collect()
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
        .w(px(ITEM_WIDTH))
        .h(px(ITEM_HEIGHT))
        .flex()
        .flex_none()
        .flex_col()
        .items_center()
        .justify_center()
        .overflow_hidden()
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
