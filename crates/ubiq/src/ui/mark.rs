//! Ubiq's brand mark, and the one thing it does.
//!
//! The mark is what an empty page shows instead of furniture: a surface waiting for a file, a
//! conversation or a document says so with the mark that owns the window rather than with a button
//! nobody asked for. Five screens draw it, so it is a module rather than a private helper on the
//! one that had it first.
//!
//! **Two layers, because one of them moves.** The ring is an `svg()` and the cubes are an `img()`,
//! and that split is forced: GPUI can only transform an `svg()`, and it draws one by rasterising
//! the markup to an alpha mask and tinting it with `text_color` — a single colour, which is exactly
//! what the ring is and exactly what the cubes are not (three face shades apiece). So the ring is
//! one file for both themes with [`theme::mark`] deciding its colour, the cubes are a file per
//! theme drawn in full colour, and only the ring can spin.
//!
//! **The layers are put back together by arithmetic, not by luck.** A rotation turns an element
//! about its own bounds' centre, so the ring's file is cut with its box centred on the ring rather
//! than on the mark — its viewBox carries the offset — and [`RING_DX`]/[`RING_DY`] here shift that
//! box back over the cubes. Change one and the mark comes apart.

use std::f32::consts::TAU;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, ClickEvent, Context, Image, ImageFormat, ImageSource,
    InteractiveElement as _, IntoElement, ParentElement, StatefulInteractiveElement as _, Styled,
    Transformation, div, img, px, radians, size, svg,
};

use crate::app::AppState;
use crate::theme::{self, Mode};

/// The ring, alone, with its box centred on itself. One file: an `svg()` is an alpha mask, so the
/// theme picks the colour and there is nothing per-theme left in the markup.
const RING: &[u8] = include_bytes!("../../../../assets/logo-ring.svg");
/// The two cubes, in the mark's own `0 0 1024 1024` box. A file per theme, because these are drawn
/// in full colour and each cube's three faces are three shades.
const CUBES_WHITE: &[u8] = include_bytes!("../../../../assets/logo-white-cubes.svg");
const CUBES_BLUE: &[u8] = include_bytes!("../../../../assets/logo-blue-cubes.svg");

/// How big the mark is drawn, in logical pixels. One size everywhere it appears.
const SIZE: f32 = 200.;
/// The side of the mark's own coordinate box, which both files share.
const VIEWBOX: f32 = 1024.;
/// Where the ring's box sits relative to the cubes', in logical pixels: the ring file's viewBox
/// origin, scaled. `logo-ring.svg` states `viewBox="-29.25 7.65 1024 1024"` so that the ring's
/// centre is its box's centre; undoing that origin here is what puts the mark back together.
const RING_DX: f32 = -29.25 * (SIZE / VIEWBOX);
const RING_DY: f32 = 7.65 * (SIZE / VIEWBOX);

/// The big cube's bounds in the mark's box, from `logo-*-cubes.svg`'s `cube` group. The triple
/// click that spins the mark is taken here rather than over the whole mark: the cube is the thing
/// in the middle, and a click on the empty corners of a 200-pixel square is not a click on it.
const CUBE_LEFT: f32 = 356.72 * (SIZE / VIEWBOX);
const CUBE_TOP: f32 = 373.64 * (SIZE / VIEWBOX);
const CUBE_WIDTH: f32 = (608.55 - 356.72) * (SIZE / VIEWBOX);
const CUBE_HEIGHT: f32 = (659.12 - 373.64) * (SIZE / VIEWBOX);

/// How solid the mark is when it is the only thing on the page.
const ALONE: f32 = 0.5;
/// …and when a page's own words are over it. Far fainter, because a watermark that competes with
/// the sentence in front of it has stopped being a background.
const BEHIND: f32 = 0.16;

/// How long a spin takes, end to end — the wind-up, the two turns and the settle.
pub const SPIN: Duration = Duration::from_millis(2_400);

/// The share of the run spent winding up, and how far back it winds, in turns.
const WIND: f32 = 0.16;
const WIND_TURNS: f32 = -0.04;
/// How far the ring travels once released. Two full turns, so it ends where it started.
const TURNS: f32 = 2.0;
/// How much bigger the mark gets while it spins, and when it starts shrinking back.
const LIFT: f32 = 0.12;
const LIFT_OUT: f32 = 0.72;

/// The mark, alone on an empty page: nothing else, nothing around it.
pub fn alone(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .items_center()
        .justify_center()
        .bg(theme::app_bg())
        .child(mark(app, ALONE, cx))
        .into_any_element()
}

/// A page's own empty state, with the mark behind it.
///
/// The mark is painted under `content` rather than beside it, so a screen keeps the words it
/// already had and gains a background. Both fill the same box; the mark is faint enough that what
/// is written over it still reads.
pub fn backdrop(app: &AppState, content: AnyElement, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .relative()
        .flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::app_bg())
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(mark(app, BEHIND, cx)),
        )
        .child(
            div()
                .relative()
                .flex()
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .child(content),
        )
        .into_any_element()
}

/// The mark itself: the ring, the cubes over it, and the cube's hit area over both.
fn mark(app: &AppState, opacity: f32, cx: &mut Context<AppState>) -> AnyElement {
    let cubes = match app.workbench.theme_id.mode() {
        Mode::Light => CUBES_BLUE,
        Mode::Dark => CUBES_WHITE,
    };
    let image = Arc::new(Image::from_bytes(ImageFormat::Svg, cubes.to_vec()));
    div()
        .relative()
        .size(px(SIZE))
        .opacity(opacity)
        .child(
            div()
                .absolute()
                .left(px(RING_DX))
                .top(px(RING_DY))
                .size(px(SIZE))
                .child(ring(app)),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .child(img(ImageSource::Image(image)).size_full()),
        )
        .child(
            div()
                .id("mark-cube")
                .absolute()
                .left(px(CUBE_LEFT))
                .top(px(CUBE_TOP))
                .w(px(CUBE_WIDTH))
                .h(px(CUBE_HEIGHT))
                .on_click(cx.listener(|this, event: &ClickEvent, _window, cx| {
                    if event.click_count() >= 3 {
                        this.spin_mark(cx);
                    }
                })),
        )
        .into_any_element()
}

/// The ring layer, still or spinning.
///
/// **Two different elements, not one element with a flag.** A one-shot animation plays when the
/// element carrying it first appears, so a ring that always carried one would spin every time the
/// user opened an empty page. The animated ring exists only while a spin is running, and its id
/// carries the spin's number so that asking for a second one mid-spin restarts it.
fn ring(app: &AppState) -> AnyElement {
    let ring = svg().size_full().text_color(theme::mark()).data(RING);
    let mark = &app.workbench.mark;
    if !mark.spinning {
        return ring.into_any_element();
    }
    ring.with_animation(
        gpui::ElementId::NamedInteger("mark-ring".into(), mark.spin),
        Animation::new(SPIN),
        |ring, t| {
            let lift = grow(t);
            ring.with_transformation(
                Transformation::rotate(radians(turns(t) * TAU)).with_scaling(size(lift, lift)),
            )
        },
    )
    .into_any_element()
}

/// How far round the ring is, in turns, a fraction `t` of the way through the spin.
///
/// Wind, release, settle. It pulls back a fraction of a turn first, because a thing that is about
/// to move fast reads as heavy only if something loads it; then it lets go and runs two full turns,
/// arriving about seventeen degrees past the mark and walking back onto it rather than stopping
/// dead. Two turns exactly, so the ring finishes where it started and the still ring that replaces
/// it is in the same place.
fn turns(t: f32) -> f32 {
    if t <= WIND {
        let u = t / WIND;
        // Out to the wound position and held there: eased to a stop, so the release is a release
        // rather than a bounce.
        return WIND_TURNS * (1.0 - (1.0 - u) * (1.0 - u));
    }
    let v = (t - WIND) / (1.0 - WIND) - 1.0;
    // The back-out cubic, with its overshoot turned down from the usual ten percent to two: ten
    // percent of two turns is most of another one, which is a second spin and not an arrival.
    let arc = 1.0 + 2.0 * v * v * v + v * v;
    WIND_TURNS + (TURNS - WIND_TURNS) * arc
}

/// How much bigger the mark is a fraction `t` of the way through the spin.
///
/// It swells as it winds up, holds while it turns, and is back to its own size by the end — the
/// same shape as something lifting off a surface to move and settling back onto it.
fn grow(t: f32) -> f32 {
    1.0 + LIFT * (ramp(0., WIND, t) - ramp(LIFT_OUT, 1., t))
}

/// The smoothstep: 0 below `from`, 1 above `to`, and eased in between.
fn ramp(from: f32, to: f32, t: f32) -> f32 {
    let u = ((t - from) / (to - from)).clamp(0., 1.);
    u * u * (3.0 - 2.0 * u)
}
