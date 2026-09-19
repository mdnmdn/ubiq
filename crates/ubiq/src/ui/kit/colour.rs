//! The colour picker: a saturation/value plane, a hue strip, a preview and a hex field.
//!
//! **The swatch surface is content, not a design decision.** The rule that no colour is written
//! outside `theme.rs` holds for the picker's *chrome* — its borders, its cursor box, the field it
//! prints the hex into — and the generated HSV values pass through it the same way a scene's stroke
//! or the sixteen ANSI terminal colours do. They are the thing being picked.
//!
//! The control knows nothing about what is being coloured: it is told a hue, a saturation and a
//! value, the colour that is currently in force, and what to call when one of its cells is clicked.
//! Project settings is the first caller; the theme editor is the second.

use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Bounds, ElementId, Entity, InteractiveElement, IntoElement, ParentElement,
    Rgba, StatefulInteractiveElement, Styled, Window, canvas, div, fill, point, px, size,
};
use gpui_component::input::{Input, InputState};

use crate::theme;
use crate::ui::kit::field;

/// A pick the kit can raise without knowing which view owns it. Call sites build one with
/// `ui::hsv`.
pub type HsvAction = Rc<dyn Fn(f32, f32, f32, &mut Window, &mut App)>;

/// The plane's cell grid. Content geometry: the number of cells is what the surface *is*, so it
/// does not move with the UI scale.
const SV_COLS: usize = 16;
const SV_ROWS: usize = 10;
const HUE_STEPS: usize = 24;
const CELL: f32 = 12.0;

/// The full HSV surface.
///
/// `prefix` namespaces every cell's element id, so two pickers can be on screen at once.
/// `hex` is the caller's field — the caller owns it, prints the current colour into it and reads
/// what was typed back out, exactly as it does for every other kit control that takes one.
pub fn colour_picker(
    prefix: &str,
    hue: f32,
    sat: f32,
    val: f32,
    current: Rgba,
    hex: &Entity<InputState>,
    hex_focused: bool,
    on_pick: HsvAction,
) -> AnyElement {
    let rows: Vec<AnyElement> = (0..SV_ROWS)
        .map(|row| {
            let cells: Vec<AnyElement> = (0..SV_COLS)
                .map(|col| {
                    let s = col as f32 / (SV_COLS - 1) as f32;
                    let v = 1.0 - row as f32 / (SV_ROWS - 1) as f32;
                    let pick = on_pick.clone();
                    div()
                        .id(ElementId::Name(format!("{prefix}-sv-{col}-{row}").into()))
                        .size(px(CELL))
                        .flex_none()
                        .cursor_pointer()
                        .on_click(move |_, window, cx| pick(hue, s, v, window, cx))
                        .into_any_element()
                })
                .collect();
            div().flex().flex_none().children(cells).into_any_element()
        })
        .collect();

    let hues: Vec<AnyElement> = (0..HUE_STEPS)
        .map(|step| {
            let h = step as f32 / (HUE_STEPS - 1) as f32;
            let pick = on_pick.clone();
            div()
                .id(ElementId::Name(format!("{prefix}-hue-{step}").into()))
                .w(px(CELL))
                .h(px(theme::scaled(14.)))
                .flex_none()
                .cursor_pointer()
                .bg(theme::rgba_of(theme::hsv_to_rgb(h, 1.0, 1.0)))
                .when((h - hue).abs() < 0.5 / HUE_STEPS as f32, |this| {
                    this.border_1().border_color(theme::text())
                })
                .on_click(move |_, window, cx| pick(h, sat, val, window, cx))
                .into_any_element()
        })
        .collect();

    div()
        .flex()
        .flex_col()
        .gap_2()
        .pt_1()
        .child(
            div()
                .flex()
                .gap_3()
                .child(
                    div()
                        .relative()
                        .w(px(CELL * SV_COLS as f32))
                        .h(px(CELL * SV_ROWS as f32))
                        .child(sv_plane(hue))
                        .child(div().absolute().inset_0().flex().flex_col().children(rows))
                        .child(sv_mark(sat, val)),
                )
                .child(
                    div()
                        .w(px(theme::scaled(36.)))
                        .h(px(CELL * SV_ROWS as f32))
                        .flex_none()
                        .bg(current)
                        .border_1()
                        .border_color(theme::border()),
                ),
        )
        .child(div().flex().children(hues))
        .child(
            field(theme::border(), hex_focused)
                .px_2()
                .w(px(theme::scaled(140.)))
                .h(px(theme::scaled(30.)))
                .items_center()
                .child(Input::new(hex).appearance(false)),
        )
        .into_any_element()
}

/// The continuous saturation/value wash the clickable cells sit over.
///
/// Painted in 4px squares rather than per pixel: the grid above it quantises the pick anyway, so
/// the wash only has to read as continuous.
fn sv_plane(hue: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<gpui::Pixels>, _, window, _| {
            let w = f32::from(bounds.size.width).max(1.0);
            let h = f32::from(bounds.size.height).max(1.0);
            let step = 4.0;
            let mut y = 0.0;
            while y < h {
                let mut x = 0.0;
                while x < w {
                    let sat = (x / w).clamp(0.0, 1.0);
                    let val = 1.0 - (y / h).clamp(0.0, 1.0);
                    window.paint_quad(fill(
                        Bounds::new(
                            bounds.origin + point(px(x), px(y)),
                            size(px(step), px(step)),
                        ),
                        theme::rgba_of(theme::hsv_to_rgb(hue, sat, val)),
                    ));
                    x += step;
                }
                y += step;
            }
        },
    )
    .size_full()
}

/// Which cell is picked, drawn as a box on the plane. Chrome, so it is a token.
fn sv_mark(sat: f32, val: f32) -> impl IntoElement {
    div()
        .absolute()
        .left(px((sat * (SV_COLS as f32 - 1.0) * CELL).round()))
        .top(px(((1.0 - val) * (SV_ROWS as f32 - 1.0) * CELL).round()))
        .size(px(CELL))
        .border_1()
        .border_color(theme::text())
}
