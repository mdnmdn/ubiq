//! The markdown viewer header's customisation popover — proposal §12
//! (`_docs/inbox/markdown-improvement-proposal.md`), T-118.
//!
//! One control, `kit::popover`, holding the four things the proposal's control panel asks for:
//! the width preset and density T-116 drew as loose pills in the header (folded in here, and
//! removed from the header itself), plus the minimap's visibility and side, generalised from the
//! plan modal's own `plan_minimap` flag so the standard viewer and the plan surface share one
//! setting. Everything on the panel persists through `UiSettings` (`AppState::remember_settings`)
//! — the same path `plan_minimap` already used.

use gpui::{
    AnyElement, Context, ElementId, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::app::AppState;
use crate::state::MenuId;
use crate::theme::{MdDensity, MdMinimapSide, MdWidth};
use crate::ui::kit::{UbiqIcon, check_box, choice_pill, icon_button, popover, section_label};
use crate::ui::{eid2, handler};

/// How wide the popover draws — wide enough for the widest pill row (three width presets) on one
/// line at the default scale.
const PANEL_WIDTH: f32 = 220.;

/// Whether the popover is up.
pub fn is_open(app: &AppState) -> bool {
    app.workbench.open_menu == Some(MenuId::MdOptions)
}

/// The header's trigger: an icon-only button — tooltip carries the meaning, per the kit's own
/// rule for icon-only controls — that opens the panel beside it when lit.
pub fn control(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let open = is_open(app);
    let mut trigger = icon_button(
        "md-options-control",
        UbiqIcon::TitlebarSettings,
        open,
        cx.listener(|this, _, _, cx| this.open_md_options(cx)),
    )
    .tooltip(|window, cx| {
        gpui_component::tooltip::Tooltip::new("Reading options \u{2014} width, density, minimap")
            .build(window, cx)
    });
    if open {
        trigger = trigger.child(panel(app, cx));
    }
    trigger.into_any_element()
}

/// The panel itself: width, density, then the minimap's two facets.
fn panel(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let ui = &app.workbench.settings.ui;
    let view = cx.entity();

    popover(
        ElementId::Name("md-options-popover".into()),
        px(PANEL_WIDTH),
        Some("md-options-popover"),
        Some(std::rc::Rc::new(handler(&view, |this, _, cx| {
            this.close_menu(cx)
        }))),
        vec![
            row("width", width_pills(ui.md_width, cx)),
            row("density", density_pills(ui.md_density, cx)),
            minimap_row(ui.md_minimap, cx),
            row("minimap side", side_pills(ui.md_minimap_side, cx)),
        ],
    )
}

/// One labelled row: a section label above its control, the same shape every group in the panel
/// takes.
fn row(label: &str, control: AnyElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .px_2()
        .py_1()
        .child(section_label(label))
        .child(control)
        .into_any_element()
}

fn width_pills(current: MdWidth, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .children(MdWidth::ALL.iter().copied().map(|width| {
            choice_pill(
                eid2("md-options-width", "popover", width.label()),
                width.label(),
                current == width,
                cx.listener(move |this, _, _, cx| this.set_md_width(width, cx)),
            )
        }))
        .into_any_element()
}

fn density_pills(current: MdDensity, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .children(MdDensity::ALL.iter().copied().map(|density| {
            choice_pill(
                eid2("md-options-density", "popover", density.label()),
                density.label(),
                current == density,
                cx.listener(move |this, _, _, cx| this.set_md_density(density, cx)),
            )
        }))
        .into_any_element()
}

fn side_pills(current: MdMinimapSide, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .children(MdMinimapSide::ALL.iter().copied().map(|side| {
            choice_pill(
                eid2("md-options-side", "popover", side.label()),
                side.label(),
                current == side,
                cx.listener(move |this, _, _, cx| this.set_md_minimap_side(side, cx)),
            )
        }))
        .into_any_element()
}

/// The one on/off row of the panel — a checkbox rather than a pill, since it is a single flag
/// beside a label rather than a set to pick one of.
fn minimap_row(visible: bool, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .px_2()
        .py_1()
        .child(section_label("minimap"))
        .child(check_box(
            "md-options-minimap",
            visible,
            cx.listener(|this, _, _, cx| this.toggle_md_minimap(cx)),
        ))
        .into_any_element()
}
