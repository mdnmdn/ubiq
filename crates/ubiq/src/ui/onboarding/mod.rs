//! The onboarding wizard: its dialog, and the outline and callout a highlight step shows instead.
//!
//! **The dialog has no way out but its footer.** It is `kit::modal_locked` — no ×, no outside
//! click — and `AppState::cancel_dialog` swallows Escape while it is up; Skip is always offered,
//! so the user is never trapped. The footer is the same on the dialog and on the callout: Skip on
//! the left, a dot per step of the whole run in the middle, Back and Next (Finish on the last step)
//! on the right, each faded and inert while its step says so.
//!
//! **A highlight step steps the dialog aside.** The window stays usable: the layer over it takes
//! no mouse, and only the callout card occludes. The target's rectangle is read from the last
//! complete frame's `ui::ident` record — a frame behind, which a still target never shows — and a
//! target not on screen leaves the card centred with nothing outlined.

pub mod flows;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, Window, anchored, deferred, div, point, px,
};

use crate::app::AppState;
use crate::state::onboarding::{FormKind, OnboardingRun, StepBody};
use crate::state::ui_id::MarkRect;
use crate::theme::{self, Family, Role};
use crate::ui::kit::{choice_pill, ghost_button, modal_locked, modal_note, primary_button};
use crate::ui::{ident, viewer};

/// The wizard, or nothing while no run is open. Called from the window root.
pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(run) = app.workbench.onboarding.as_ref() else {
        return div().into_any_element();
    };
    if run.hidden() {
        callout_layer(app, run, window, cx)
    } else {
        dialog(app, run, window, cx)
    }
}

fn dialog(
    app: &AppState,
    run: &OnboardingRun,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let step = run.current();
    let body = match &step.body {
        StepBody::Markdown(source) => {
            let key = format!("onboarding.{}.{}", run.current_flow(), run.index);
            let follow = crate::ui::on_link(cx.entity(), None);
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .child(viewer::markdown::render_linked(
                    app, &key, source, false, follow, cx,
                ))
                .into_any_element()
        }
        StepBody::Form(FormKind::Palette) => palette_form(app, cx),
        StepBody::Highlight { .. } => div().into_any_element(),
    };
    modal_locked(
        "onboarding",
        theme::accent(),
        theme::onboarding_width(),
        Some(theme::onboarding_height()),
        &step.title,
        body,
        footer(app, run, cx).flex_1().into_any_element(),
        window,
    )
}

/// The first form: the palette the window wears, one pill per built-in, applied on the spot.
fn palette_form(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let current = app.workbench.theme_id;
    let pills = theme::ThemeId::all().map(|id| {
        choice_pill(
            ElementId::Name(format!("onboarding-palette-{}", id.slug()).into()),
            id.name(),
            id == current,
            cx.listener(move |this, _, _, cx| this.set_palette(id, cx)),
        )
        .into_any_element()
    });
    div()
        .pt_3()
        .flex()
        .flex_col()
        .gap_3()
        .child(modal_note(
            "Pick the colours the window wears. It changes as you click, and Settings → \
             Appearance has the accent, the text brightness and your own themes.",
        ))
        .child(div().flex().flex_wrap().gap_1().children(pills))
        .into_any_element()
}

/// Skip, the dots, Back and Next — shared by the dialog and the callout.
fn footer(app: &AppState, run: &OnboardingRun, cx: &mut Context<AppState>) -> gpui::Div {
    let back_open = app.onboarding_back_open(cx);
    let next_open = app.onboarding_next_open(cx);
    let (index, len) = run.progress();
    let dots = (0..len).map(|i| {
        let colour = if i == index {
            theme::accent()
        } else if i < index {
            theme::accent_muted()
        } else {
            theme::text_faint()
        };
        div()
            .size(px(theme::onboarding_dot()))
            .flex_none()
            .rounded_full()
            .bg(colour)
    });

    div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            "onboarding-skip",
            None,
            "Skip",
            cx.listener(|this, _, _, cx| this.skip_onboarding(cx)),
        ))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_wrap()
                .items_center()
                .justify_center()
                .gap_1p5()
                .children(dots),
        )
        .child(
            ghost_button(
                "onboarding-back",
                None,
                "Back",
                cx.listener(|this, _, _, cx| this.onboarding_back(cx)),
            )
            .when(!back_open, |button| button.opacity(0.5)),
        )
        .child(
            primary_button(
                "onboarding-next",
                None,
                if run.is_last() { "Finish" } else { "Next" },
                cx.listener(|this, _, _, cx| this.onboarding_next(cx)),
            )
            .when(!next_open, |button| button.opacity(0.5)),
        )
}

/// A highlight step: the target outlined and the callout beside it, over a window left usable.
fn callout_layer(
    app: &AppState,
    run: &OnboardingRun,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let step = run.current();
    let StepBody::Highlight { target, text } = &step.body else {
        return div().into_any_element();
    };
    let viewport = window.viewport_size();
    let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
    let rect = ident::recorded(window)
        .into_iter()
        .rev()
        .find(|mark| &mark.id == target)
        .map(|mark| mark.rect)
        .filter(|rect| rect.width > 0. && rect.height > 0.);
    let gap = theme::help_balloon_gap();
    let width = theme::onboarding_callout_width().min(vw - 2. * gap);
    let (left, top) = place_callout(rect, width, vw, vh);

    let card = div()
        .id("onboarding-callout")
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(width))
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .bg(theme::surface_raised())
        .border_l(px(theme::accent_edge()))
        .border_color(theme::accent())
        .shadow_lg()
        // The card alone takes the mouse: the rest of the window is meant to be used.
        .occlude()
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text())
                .child(step.title.clone()),
        )
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text_muted())
                .child(text.clone()),
        )
        .child(footer(app, run, cx).pt_1());

    let layer = div()
        .w(px(vw))
        .h(px(vh))
        .relative()
        .when_some(rect, |layer, rect| layer.child(outline(rect)))
        .child(card);

    deferred(anchored().position(point(px(0.), px(0.))).child(layer))
        // The kit's modals' rung: above the panels and dropdowns, under in-place help.
        .priority(2)
        .into_any_element()
}

/// The target's outline: an accent border just outside its rectangle, drawn over the window but
/// taking no mouse.
fn outline(rect: MarkRect) -> AnyElement {
    let pad = 2.;
    div()
        .absolute()
        .left(px(rect.x - pad))
        .top(px(rect.y - pad))
        .w(px(rect.width + 2. * pad))
        .h(px(rect.height + 2. * pad))
        .border_2()
        .border_color(theme::accent())
        .into_any_element()
}

/// Where the callout goes: right of the target, else below, else left, else above, else centred
/// over it — and centred in the window when the target is not on screen. Always clamped inside
/// the window. Public so the arithmetic can be tested without one.
pub fn place_callout(rect: Option<MarkRect>, width: f32, vw: f32, vh: f32) -> (f32, f32) {
    let gap = theme::help_balloon_gap();
    let room = theme::onboarding_callout_room();
    let clamp_x = |x: f32| x.clamp(gap, (vw - width - gap).max(gap));
    let clamp_y = |y: f32| y.clamp(gap, (vh - room - gap).max(gap));

    let Some(r) = rect else {
        return (clamp_x((vw - width) / 2.), clamp_y((vh - room) / 2.));
    };
    let right = r.x + r.width + gap;
    if right + width <= vw - gap {
        return (right, clamp_y(r.y));
    }
    let below = r.y + r.height + gap;
    if below + room <= vh - gap {
        return (clamp_x(r.x), below);
    }
    let left = r.x - width - gap;
    if left >= gap {
        return (left, clamp_y(r.y));
    }
    let above = r.y - room - gap;
    if above >= gap {
        return (clamp_x(r.x), above);
    }
    (
        clamp_x(r.x + (r.width - width) / 2.),
        clamp_y(r.y + (r.height - room) / 2.),
    )
}
