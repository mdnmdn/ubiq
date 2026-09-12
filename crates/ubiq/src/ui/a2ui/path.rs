//! An icon an agent drew, painted rather than rasterised.
//!
//! The basic catalog lets an `Icon`'s `name` be `{"svgPath": "M4 12 L10 18 …"}` instead of one of
//! its 59 names — a glyph the agent supplies because the renderer has no name for it. This is that
//! arm, and it takes the other of the two routes `Svg` takes: the `d` string goes through
//! [`crate::state::a2ui::path::parse`] and is painted with `canvas()` and `PathBuilder`, the way
//! `ui/viewer/scene.rs` paints a scene and `ui/kit/canvas.rs` paints the graph's links.
//!
//! **This route needs no sanitizer, and that is the reason both routes exist.** A path `d` string
//! can express geometry and nothing else — there is no attribute in one, no element, no reference
//! and no URL — so the whole question `Svg` has to answer about what markup may reach the
//! rasteriser simply does not arise. It is also the route that can be tinted: the painter chooses
//! the colour, because an `Icon` carries none, so a drawn glyph sits in the palette beside the
//! named icons it stands among.

use gpui::{
    AnyElement, Bounds, ElementId, InteractiveElement, IntoElement, ParentElement, Pixels, Point,
    Rgba, Styled, Window, canvas, div, point, px,
};

use crate::state::a2ui::path::{P, Seg, parse};

use super::note;

/// The box a drawn glyph occupies, matching `Size::Small` — the size the named-icon arm draws at,
/// because the two sit in the same rows.
const GLYPH: f32 = 16.0;

/// The coordinate space a `d` string is assumed to be in when nothing says otherwise.
///
/// Every icon in Ubiq's own set is drawn on this box and so is Material's, which is where the
/// catalog's 59 names come from — so it is the space an agent writing one will have used.
const VIEW_BOX: f32 = 24.0;

/// Paint one `svgPath` glyph, or say what stopped the parse.
pub fn draw(id: ElementId, d: &str, colour: Rgba) -> AnyElement {
    let segments = match parse(d) {
        Ok(segments) => segments,
        Err(reason) => return note(format!("svgPath: {reason}")),
    };

    div()
        .id(id)
        .flex_none()
        .size(px(GLYPH))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| paint(&segments, bounds, colour, window),
            )
            .size_full(),
        )
        .into_any_element()
}

/// The segments, scaled out of the path's own space and into the box this element landed in.
fn paint(segments: &[Seg], bounds: Bounds<Pixels>, colour: Rgba, window: &mut Window) {
    let scale = f32::from(bounds.size.width).min(f32::from(bounds.size.height)) / VIEW_BOX;
    let origin = bounds.origin;
    let place = |p: P| -> Point<Pixels> { origin + point(px(p.x * scale), px(p.y * scale)) };

    let mut builder = gpui::PathBuilder::fill();
    // A `d` may open several subpaths, and a fill wants each closed — an unclosed subpath is
    // filled to its own start either way, so closing on every `Move` costs nothing and keeps the
    // common "two shapes, one path" glyph from being filled as one.
    let mut open = false;
    for segment in segments {
        match segment {
            Seg::Move(to) => {
                if open {
                    builder.close();
                }
                builder.move_to(place(*to));
                open = true;
            }
            Seg::Line(to) => builder.line_to(place(*to)),
            Seg::Quad(ctrl, to) => builder.curve_to(place(*to), place(*ctrl)),
            Seg::Cubic(a, b, to) => builder.cubic_bezier_to(place(*to), place(*a), place(*b)),
            Seg::Close => {
                builder.close();
                open = false;
            }
        }
    }
    if open {
        builder.close();
    }

    // A path that will not tessellate is a glyph that does not draw, which is the same outcome as
    // an unmapped icon name and is not worth a second failure mode on screen.
    if let Ok(path) = builder.build() {
        window.paint_path(path, colour);
    }
}
