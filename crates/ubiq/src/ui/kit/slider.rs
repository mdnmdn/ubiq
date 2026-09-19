//! The one continuous control: a house-skinned wrapper over the component library's slider.
//!
//! `kit::meter` only reports and `kit::stepper` only counts, so a value the user *drags* had no
//! control at all until this one. It is deliberately a wrapper rather than a control of our own:
//! `gpui_component::slider` already owns the pointer capture, the drag, the keyboard and the
//! accessibility role, and none of that is worth rewriting for a skin. What the kit adds is the
//! palette, the two icon slots that say which end is which, and a step so a drag lands on a ladder
//! rather than on 1.0374.

use gpui::prelude::FluentBuilder;
use gpui::{
    App, ElementId, Entity, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::slider::{Slider as LibrarySlider, SliderState};
use gpui_component::{Icon, Sizable as _, Size};

use crate::theme;

/// A [`SliderState`] quantised to `stops` evenly spaced values across `min..max`.
///
/// The ladder is the point. A continuous axis dragged with a mouse lands on whatever pixel the
/// pointer was over, and a preference the user cannot say out loud — "a bit bigger" — is worth
/// less than one they can return to exactly. Nine stops over a range is about the density where
/// every stop is reachable without fighting the pointer.
///
/// The library quantises to multiples of the step measured from zero, not from `min`, so a range
/// whose `min` is not itself a multiple of the step reaches its ends by the clamp rather than by
/// the ladder. Choose `min`, `max` and `stops` so the step divides them, and the two agree.
pub fn slider_state(min: f32, max: f32, stops: usize, value: f32) -> SliderState {
    let steps = stops.max(2) - 1;
    SliderState::new()
        .min(min)
        .max(max)
        .step((max - min) / steps as f32)
        .default_value(value)
}

/// A value dragged along a track, flanked by the icons that say what its two ends mean.
///
/// It carries no number and no unit: the icons are the scale. That makes it an unlabelled control,
/// so the tooltip is not optional and [`Slider::new`] takes it.
#[derive(IntoElement)]
pub struct Slider {
    id: ElementId,
    state: Entity<SliderState>,
    tooltip: SharedString,
    leading: Option<Icon>,
    trailing: Option<Icon>,
    disabled: bool,
}

impl Slider {
    /// A slider over `state`, with the tooltip that says what it moves.
    pub fn new(
        id: impl Into<ElementId>,
        state: &Entity<SliderState>,
        tooltip: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            tooltip: tooltip.into(),
            leading: None,
            trailing: None,
            disabled: false,
        }
    }

    /// The mark at the low end of the track.
    pub fn leading(mut self, icon: impl Into<Icon>) -> Self {
        self.leading = Some(icon.into());
        self
    }

    /// The mark at the high end of the track.
    pub fn trailing(mut self, icon: impl Into<Icon>) -> Self {
        self.trailing = Some(icon.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// An end mark: faint, and at the scaled small icon size like every other inline glyph.
fn end_icon(icon: Icon) -> impl IntoElement {
    icon.with_size(Size::Size(theme::icon_sm()))
        .text_color(theme::text_faint())
}

impl RenderOnce for Slider {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tip = self.tooltip;

        div()
            .id(self.id)
            .flex()
            .items_center()
            .gap_2()
            .when_some(self.leading, |this, icon| this.child(end_icon(icon)))
            .child(
                div().flex_1().min_w(px(0.)).child(
                    LibrarySlider::new(&self.state)
                        .disabled(self.disabled)
                        // The library reads the track's fill out of `background` and the thumb's
                        // out of `text`, which is how the palette reaches a control we do not
                        // draw. Squareness is not set here: `theme::dress_component_library`
                        // zeroes the library's radius, so the track and the thumb are square for
                        // the same reason every other library widget is.
                        .bg(theme::accent())
                        .text_color(theme::text()),
                ),
            )
            .when_some(self.trailing, |this, icon| this.child(end_icon(icon)))
            .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
            })
    }
}
