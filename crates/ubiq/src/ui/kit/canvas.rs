//! What gets painted rather than laid out: the dotted ground a graph sits on, the curves between
//! its nodes, and the trail a dragged thing leaves behind.
//!
//! Each of these is one absolutely-positioned layer filling its parent, so a caller stacks them in
//! the order they should read — ground, links, cards, sand — and none of them takes a click.
//!
//! Everything here is view-agnostic and takes plain geometry in window coordinates. Nothing in it
//! knows what a card or an agent is.

use gpui::{
    Bounds, IntoElement, ParentElement, Pixels, Point, Rgba, Styled, canvas, div, fill, point, px,
    size,
};

use crate::theme;

/// A layer that fills its parent, sits under or over its siblings, and never takes a click. The
/// parent needs `.relative()`.
fn layer() -> gpui::Div {
    div().absolute().inset_0().size_full()
}

/// The dotted ground. `spacing` is already scaled by the caller's zoom, so the grid breathes with
/// the graph rather than staying a fixed mesh the cards slide over.
pub fn dot_grid(spacing: f32, offset: Point<f32>) -> impl IntoElement {
    let spacing = spacing.max(8.0);
    let colour = theme::fade(theme::text_faint(), 0.35);

    layer().child(canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            // Start on the first dot at or before the top-left corner, so scrolling the graph
            // moves the dots with it instead of resampling the mesh.
            let start_x = -(offset.x.rem_euclid(spacing));
            let start_y = -(offset.y.rem_euclid(spacing));

            let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            let mut y = start_y;
            while y < h {
                let mut x = start_x;
                while x < w {
                    window.paint_quad(fill(
                        Bounds::new(bounds.origin + point(px(x), px(y)), size(px(1.5), px(1.5))),
                        colour,
                    ));
                    x += spacing;
                }
                y += spacing;
            }
        },
    ))
}

/// One curve between two points, in the colour of whatever it connects.
pub struct Link {
    pub from: Point<f32>,
    pub to: Point<f32>,
    pub colour: Rgba,
}

/// The curves between nodes.
///
/// Each is a cubic drawn as segments — the same device the progress ring uses — with its control
/// points pulled vertically, so a link leaves its parent downwards and arrives at its child from
/// above however far apart the two are.
pub fn links(links: Vec<Link>) -> impl IntoElement {
    layer().child(canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            for link in &links {
                let origin: Point<Pixels> = bounds.origin;
                let (a, b) = (link.from, link.to);
                let lift = ((b.y - a.y).abs() * 0.5).clamp(24.0, 90.0);
                let (c1, c2) = (point(a.x, a.y + lift), point(b.x, b.y - lift));

                let mut path = gpui::PathBuilder::stroke(px(1.25));
                let steps = 24;
                for step in 0..=steps {
                    let t = step as f32 / steps as f32;
                    let u = 1.0 - t;
                    let cubic = |p0: f32, p1: f32, p2: f32, p3: f32| {
                        u * u * u * p0
                            + 3.0 * u * u * t * p1
                            + 3.0 * u * t * t * p2
                            + t * t * t * p3
                    };
                    let p = origin
                        + point(
                            px(cubic(a.x, c1.x, c2.x, b.x)),
                            px(cubic(a.y, c1.y, c2.y, b.y)),
                        );
                    if step == 0 {
                        path.move_to(p);
                    } else {
                        path.line_to(p);
                    }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, link.colour);
                }
            }
        },
    ))
}

/// One grain of a drag trail, as the painter wants it: where it is, how far through its life it
/// is, and how big it started.
pub struct Grain {
    pub at: Point<f32>,
    /// Zero when it landed, one when it is gone.
    pub age: f32,
    pub size: f32,
}

/// The trail a dragged thing leaves.
///
/// Grains are squares, not dots, for the same reason nothing else in Ubiq is round: the shape is
/// the house's, and a spray of tiny squares reads as material coming off the thing being carried.
/// They shrink and fade on the same curve, so the trail thins towards its tail rather than
/// vanishing all at once.
pub fn sand(grains: Vec<Grain>, colour: Rgba) -> impl IntoElement {
    layer().child(canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            for grain in &grains {
                let left = 1.0 - grain.age;
                // Cubed, so most of a grain's visible life is spent near full strength and the
                // tail goes quickly — which is what makes the trail read as a trail.
                let alpha = left * left * left;
                let side = (grain.size * (0.35 + left * 0.65)).max(1.0);
                // A grain drifts a little as it dies, so the trail settles rather than freezing.
                let drift = grain.age * grain.size * 1.6;
                window.paint_quad(fill(
                    Bounds::new(
                        bounds.origin + point(px(grain.at.x), px(grain.at.y + drift)),
                        size(px(side), px(side)),
                    ),
                    theme::fade(colour, alpha),
                ));
            }
        },
    ))
}

/// The box a task's cards sit in: a dashed outline, drawn rather than bordered because GPUI's
/// borders are solid and the dash is what says the box is a grouping and not a surface.
///
/// `active` is what a drop target lights up as while a card is over it: a longer dash and a
/// heavier stroke, so the box the card would land in is the one that changed.
pub fn dashed_box(rect: (f32, f32, f32, f32), colour: Rgba, active: bool) -> impl IntoElement {
    let (dash, gap, stroke) = if active {
        (11.0, 4.0, 1.6)
    } else {
        (6.0, 5.0, 1.0)
    };

    layer().child(canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (x, y, w, h) = rect;
            let o = bounds.origin;
            let corner = |dx: f32, dy: f32| o + point(px(x + dx), px(y + dy));

            let mut path = gpui::PathBuilder::stroke(px(stroke)).dash_array(&[px(dash), px(gap)]);
            path.move_to(corner(0.0, 0.0));
            path.line_to(corner(w, 0.0));
            path.line_to(corner(w, h));
            path.line_to(corner(0.0, h));
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, colour);
            }
        },
    ))
}
