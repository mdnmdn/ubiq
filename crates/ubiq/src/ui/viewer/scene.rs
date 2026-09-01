//! An Excalidraw scene, painted.
//!
//! Excalidraw is **data with a closed vocabulary**, not a language, so there is nothing here to
//! delegate to: the file becomes shapes in [`crate::state::scene`], and this module walks them with
//! `canvas()` and `PathBuilder`. **The viewer is read-only.** It draws the scene and nothing else;
//! editing is not proposed and is not built.
//!
//! The scene is fitted to the panel with a margin and its aspect ratio preserved. The wheel zooms
//! about the pointer, a drag pans, a double-click restores the fit. A scene in a Markdown fence is
//! drawn at its own size instead, because a fence is a block in a document and not a panel.
//!
//! ## The limits, inherited from `_tools/excalidraw.py`
//!
//! That tool renders the same subset to SVG for the wireframes under `_docs/design/`, and
//! `_docs/tech/diagram-format.md` states what it reproduces. This painter inherits the same stated
//! limits, deliberately, so that a diagram looks the same in Ubiq as in the documentation:
//!
//! - **The hand-drawn `roughness` style renders as clean vector.** Nothing is sketched twice, no
//!   stroke wobbles, and the `hand` font family falls back to the interface's own.
//! - **Hachure and cross-hatch fills render solid.** `fillStyle` is read as "there is a fill".
//! - **Text is never rotated**, because a glyph run is painted, not transformed. Every other shape
//!   honours `angle`.
//! - **An embedded image draws as a placeholder box**, as it does in the reference renderer, which
//!   emits nothing for one. Decoding PNG and JPEG would mean a new dependency in the interface
//!   crate for a case `_docs/design/` does not contain.
//! - **An element type this painter does not know is skipped and the rest of the scene draws** —
//!   the parser drops it, and one unknown type is a missing shape rather than a blank panel.
//!
//! ## Colour
//!
//! `_docs/tech/ui-and-design.md` forbids a literal colour outside `theme.rs`, and **this module
//! breaks no part of it.** A scene's own strokes and fills are read out of the file: they are data,
//! the way a photograph's pixels are, and passing them to `paint_path` is not a design decision.
//! Every colour *this module chooses* — the placeholder an image draws as, the text of a failure
//! note — is a theme token, and there is no literal here. The ground behind a scene that names
//! none is Excalidraw's own white canvas ([`Rgba8::DEFAULT_BACKGROUND`]), which is data from the
//! format, the way a stroke colour is.

use gpui::{
    AnyElement, App, Bounds, Context, IntoElement, ParentElement, PathBuilder, Pixels, Point, Rgba,
    Styled, TextRun, Window, canvas, div, fill, point, px, size,
};

use crate::app::AppState;
use crate::state::scene::{
    Element, ElementKind, FontFamily, Rgba8, Scene, SceneError, StrokeStyle, TextAlign,
};
use crate::state::viewport::{self, Camera, Content};
use crate::theme;

/// Excalidraw's own corner radius for a rounded rectangle, matching the reference renderer's `rx`.
const CORNER: f32 = 12.0;

/// How many segments an ellipse is drawn with. Enough that the seams are invisible at any zoom the
/// fit produces, and cheap enough that a wireframe full of them costs nothing.
const ELLIPSE_STEPS: usize = 64;

pub fn render(source: &str) -> AnyElement {
    match parsed(source) {
        Ok(scene) => draw_static(scene),
        Err(note) => note,
    }
}

/// The same scene, in a panel the user can pan and zoom.
pub fn live(
    app: &AppState,
    key: &str,
    source: &str,
    cx: &mut Context<AppState>,
) -> AnyElement {
    match parsed(source) {
        Ok(scene) => draw_live(app, key, scene, cx),
        Err(note) => note,
    }
}

fn parsed(source: &str) -> Result<Scene, AnyElement> {
    match Scene::parse(source.as_bytes()) {
        Ok(scene) if scene.elements.is_empty() => {
            Err(super::note("this scene is empty", theme::text_faint()))
        }
        Ok(scene) => Ok(scene),
        // Every failure says why in the reader's own words, including the compressed case — which
        // is a scene that exists and cannot be unpacked, not a file that went wrong.
        Err(why @ SceneError::Compressed) => {
            Err(super::note(why.to_string(), theme::text_muted()))
        }
        Err(why) => Err(super::note(why.to_string(), theme::danger())),
    }
}

fn content_of(scene: &Scene) -> Content {
    Content {
        min_x: scene.bounds.min_x,
        min_y: scene.bounds.min_y,
        width: scene.bounds.width(),
        height: scene.bounds.height(),
    }
}

/// The file's canvas colour, or Excalidraw's white when it named none.
fn ground_of(scene: &Scene) -> Rgba {
    tint(
        scene.background.unwrap_or(Rgba8::DEFAULT_BACKGROUND),
        1.0,
    )
}

/// A scene in a Markdown fence: drawn at its own size, no camera of its own, because a fence is a
/// block in a document and the document is what scrolls.
fn draw_static(scene: Scene) -> AnyElement {
    let content = content_of(&scene);
    let panel_w = content.width.max(1.0) + viewport::MARGIN * 2.0;
    let panel_h = content.height.max(1.0) + viewport::MARGIN * 2.0;
    let camera = viewport::Viewport::default().camera(content, panel_w, panel_h);
    let ground = ground_of(&scene);

    div()
        .flex_none()
        .w(px(panel_w))
        .h(px(panel_h))
        .bg(ground)
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let view = View::from_camera(camera, bounds.origin);
                    for element in &scene.elements {
                        paint(element, &view, window, cx);
                    }
                },
            )
            .w(px(panel_w))
            .h(px(panel_h)),
        )
        .into_any_element()
}

fn draw_live(
    app: &AppState,
    key: &str,
    scene: Scene,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let content = content_of(&scene);
    let camera_at = app.viewport(key);
    super::viewport::surface(app, key, ground_of(&scene), content, cx)
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let camera = camera_at.camera(
                        content,
                        f32::from(bounds.size.width),
                        f32::from(bounds.size.height),
                    );
                    let view = View::from_camera(camera, bounds.origin);
                    for element in &scene.elements {
                        paint(element, &view, window, cx);
                    }
                },
            )
            .absolute()
            .inset_0()
            .size_full(),
        )
        .into_any_element()
}

// ------------------------------------------------------------------------------------------- //
// The transform
// ------------------------------------------------------------------------------------------- //

/// Scene coordinates to window coordinates: one uniform scale and one offset, so the aspect ratio
/// survives and a diagram is never stretched to fit.
struct View {
    origin: Point<Pixels>,
    scale: f32,
    offset_x: f32,
    offset_y: f32,
}

impl View {
    fn from_camera(camera: Camera, origin: Point<Pixels>) -> Self {
        Self {
            origin,
            scale: camera.scale,
            offset_x: camera.offset_x,
            offset_y: camera.offset_y,
        }
    }

    fn at(&self, x: f32, y: f32) -> Point<Pixels> {
        self.origin
            + point(
                px(self.offset_x + x * self.scale),
                px(self.offset_y + y * self.scale),
            )
    }

    /// A scene length in window pixels.
    fn len(&self, value: f32) -> f32 {
        value * self.scale
    }

    /// A stroke never thins below a hairline, or a scaled-down diagram loses its outlines.
    fn stroke_width(&self, width: f32) -> f32 {
        self.len(width).max(0.75)
    }
}

/// One point of an element, rotated about the element's own centre and then placed.
///
/// `angle` is radians in the file and applies to the shape, not to the scene, which is why it is
/// resolved here rather than folded into [`View`].
fn place(element: &Element, view: &View, x: f32, y: f32) -> Point<Pixels> {
    if element.angle == 0.0 {
        return view.at(x, y);
    }
    let (cx, cy) = (
        element.x + element.width / 2.0,
        element.y + element.height / 2.0,
    );
    let (sin, cos) = element.angle.sin_cos();
    let (dx, dy) = (x - cx, y - cy);
    view.at(cx + dx * cos - dy * sin, cy + dx * sin + dy * cos)
}

// ------------------------------------------------------------------------------------------- //
// Colour and stroke
// ------------------------------------------------------------------------------------------- //

/// A colour the file named, at the element's opacity. Data from the scene, not a token — see the
/// module header.
fn tint(colour: Rgba8, opacity: f32) -> Rgba {
    Rgba {
        r: f32::from(colour.r) / 255.0,
        g: f32::from(colour.g) / 255.0,
        b: f32::from(colour.b) / 255.0,
        a: f32::from(colour.a) / 255.0 * opacity,
    }
}

/// A stroke builder carrying the element's width and dash pattern, both scaled with the view so a
/// dashed line stays dashed at any zoom.
fn pen(view: &View, width: f32, style: StrokeStyle) -> PathBuilder {
    let builder = PathBuilder::stroke(px(view.stroke_width(width)));
    match style {
        StrokeStyle::Solid => builder,
        // The reference renderer's own `stroke-dasharray` values.
        StrokeStyle::Dashed => {
            builder.dash_array(&[px(view.len(10.0).max(2.0)), px(view.len(6.0).max(2.0))])
        }
        StrokeStyle::Dotted => {
            builder.dash_array(&[px(view.stroke_width(2.0)), px(view.len(6.0).max(2.0))])
        }
    }
}

// ------------------------------------------------------------------------------------------- //
// The elements
// ------------------------------------------------------------------------------------------- //

fn paint(element: &Element, view: &View, window: &mut Window, cx: &mut App) {
    match &element.kind {
        ElementKind::Rectangle { rounded } => rectangle(element, *rounded, view, window),
        ElementKind::Ellipse => outline(element, ellipse_points(element), view, window),
        ElementKind::Diamond => outline(element, diamond_points(element), view, window),
        ElementKind::Frame { name } => frame(element, name.as_deref(), view, window, cx),
        ElementKind::Line {
            points,
            start_arrow,
            end_arrow,
        }
        | ElementKind::Arrow {
            points,
            start_arrow,
            end_arrow,
        } => connector(element, points, *start_arrow, *end_arrow, view, window),
        ElementKind::FreeDraw { points } => connector(element, points, false, false, view, window),
        ElementKind::Text {
            text,
            font_size,
            family,
            align,
        } => label(element, text, *font_size, *family, *align, view, window, cx),
        ElementKind::Image { .. } => placeholder(element, view, window),
    }
}

/// A rectangle. Unrotated and square-cornered, it is a quad — the cheapest thing GPUI paints, and
/// what most of a wireframe is made of. Anything else goes through a path.
fn rectangle(element: &Element, rounded: bool, view: &View, window: &mut Window) {
    let radius = if rounded {
        view.len(
            CORNER
                .min(element.width.abs() / 2.0)
                .min(element.height.abs() / 2.0),
        )
    } else {
        0.0
    };

    if element.angle == 0.0 && radius <= 0.5 {
        if let Some(colour) = element.fill {
            let top_left = view.at(element.x, element.y);
            let extent = size(px(view.len(element.width)), px(view.len(element.height)));
            window.paint_quad(fill(
                Bounds::new(top_left, extent),
                tint(colour, element.opacity),
            ));
        }
        stroke_polygon(element, corner_points(element), true, view, window);
        return;
    }

    if radius <= 0.5 {
        outline(element, corner_points(element), view, window);
        return;
    }

    // Rounded, or rotated, or both: one path either way. The corners are quadratic curves with the
    // sharp corner as their control point, which is what an SVG `rx` draws.
    let round = |builder: &mut PathBuilder| {
        let (x, y, w, h) = (element.x, element.y, element.width, element.height);
        let r = CORNER.min(w.abs() / 2.0).min(h.abs() / 2.0);
        let p = |bx: f32, by: f32| place(element, view, bx, by);
        builder.move_to(p(x + r, y));
        builder.line_to(p(x + w - r, y));
        builder.curve_to(p(x + w, y + r), p(x + w, y));
        builder.line_to(p(x + w, y + h - r));
        builder.curve_to(p(x + w - r, y + h), p(x + w, y + h));
        builder.line_to(p(x + r, y + h));
        builder.curve_to(p(x, y + h - r), p(x, y + h));
        builder.line_to(p(x, y + r));
        builder.curve_to(p(x + r, y), p(x, y));
        builder.close();
    };

    if let Some(colour) = element.fill {
        let mut builder = PathBuilder::fill();
        round(&mut builder);
        if let Ok(path) = builder.build() {
            window.paint_path(path, tint(colour, element.opacity));
        }
    }
    if let Some(colour) = element.stroke {
        let mut builder = pen(view, element.stroke_width, element.stroke_style);
        round(&mut builder);
        if let Ok(path) = builder.build() {
            window.paint_path(path, tint(colour, element.opacity));
        }
    }
}

/// A closed shape given as scene-space points: filled, then stroked.
fn outline(element: &Element, points: Vec<(f32, f32)>, view: &View, window: &mut Window) {
    if let Some(colour) = element.fill {
        let mapped: Vec<Point<Pixels>> = points
            .iter()
            .map(|&(x, y)| place(element, view, x, y))
            .collect();
        let mut builder = PathBuilder::fill();
        builder.add_polygon(&mapped, true);
        if let Ok(path) = builder.build() {
            window.paint_path(path, tint(colour, element.opacity));
        }
    }
    stroke_polygon(element, points, true, view, window);
}

fn stroke_polygon(
    element: &Element,
    points: Vec<(f32, f32)>,
    closed: bool,
    view: &View,
    window: &mut Window,
) {
    let Some(colour) = element.stroke else {
        return;
    };
    let mapped: Vec<Point<Pixels>> = points
        .iter()
        .map(|&(x, y)| place(element, view, x, y))
        .collect();
    if mapped.len() < 2 {
        return;
    }
    let mut builder = pen(view, element.stroke_width, element.stroke_style);
    builder.add_polygon(&mapped, closed);
    if let Ok(path) = builder.build() {
        window.paint_path(path, tint(colour, element.opacity));
    }
}

fn corner_points(element: &Element) -> Vec<(f32, f32)> {
    let (x, y, w, h) = (element.x, element.y, element.width, element.height);
    vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h)]
}

fn diamond_points(element: &Element) -> Vec<(f32, f32)> {
    let (x, y, w, h) = (element.x, element.y, element.width, element.height);
    vec![
        (x + w / 2.0, y),
        (x + w, y + h / 2.0),
        (x + w / 2.0, y + h),
        (x, y + h / 2.0),
    ]
}

fn ellipse_points(element: &Element) -> Vec<(f32, f32)> {
    let (rx, ry) = (element.width / 2.0, element.height / 2.0);
    let (cx, cy) = (element.x + rx, element.y + ry);
    (0..ELLIPSE_STEPS)
        .map(|step| {
            let theta = std::f32::consts::TAU * step as f32 / ELLIPSE_STEPS as f32;
            (cx + rx * theta.cos(), cy + ry * theta.sin())
        })
        .collect()
}

/// A frame: a light dashed box with its name above the top-left corner, exactly as the reference
/// renderer draws one. A frame is a grouping, so it is drawn thinner than whatever is inside it.
fn frame(element: &Element, name: Option<&str>, view: &View, window: &mut Window, cx: &mut App) {
    let colour = element.stroke.unwrap_or(Rgba8::DEFAULT_STROKE);
    let mapped: Vec<Point<Pixels>> = corner_points(element)
        .into_iter()
        .map(|(x, y)| place(element, view, x, y))
        .collect();

    let mut builder = PathBuilder::stroke(px(view.stroke_width(1.0)))
        .dash_array(&[px(view.len(6.0).max(2.0)), px(view.len(4.0).max(2.0))]);
    builder.add_polygon(&mapped, true);
    if let Ok(path) = builder.build() {
        window.paint_path(path, tint(colour, element.opacity));
    }

    if let Some(name) = name.filter(|name| !name.is_empty()) {
        let size = view.len(14.0);
        if size >= 4.0 {
            let line = shape(
                name,
                size,
                FontFamily::Normal,
                tint(colour, element.opacity),
                window,
            );
            let origin = view.at(element.x, element.y) - point(px(0.), px(size * 1.4));
            let _ = line.paint(
                origin,
                px(size * 1.25),
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            );
        }
    }
}

/// A connector or a freehand stroke: a polyline through points the file stored relative to the
/// element's origin, with an arrowhead on either end that asked for one.
fn connector(
    element: &Element,
    points: &[(f32, f32)],
    start_arrow: bool,
    end_arrow: bool,
    view: &View,
    window: &mut Window,
) {
    let Some(colour) = element.stroke else {
        return;
    };
    let mapped: Vec<Point<Pixels>> = points
        .iter()
        .map(|&(dx, dy)| place(element, view, element.x + dx, element.y + dy))
        .collect();
    if mapped.len() < 2 {
        return;
    }
    let paint_colour = tint(colour, element.opacity);

    let mut builder = pen(view, element.stroke_width, element.stroke_style);
    builder.add_polygon(&mapped, false);
    if let Ok(path) = builder.build() {
        window.paint_path(path, paint_colour);
    }

    // Heads are solid triangles rather than a stroked chevron, so they read at every zoom the fit
    // produces. Their size follows the stroke, as the reference renderer's marker does.
    let head = view.len(9.0 + element.stroke_width * 1.5).max(4.0);
    if end_arrow {
        arrowhead(
            mapped[mapped.len() - 1],
            mapped[mapped.len() - 2],
            head,
            paint_colour,
            window,
        );
    }
    if start_arrow {
        arrowhead(mapped[0], mapped[1], head, paint_colour, window);
    }
}

fn arrowhead(
    tip: Point<Pixels>,
    from: Point<Pixels>,
    size: f32,
    colour: Rgba,
    window: &mut Window,
) {
    let (dx, dy) = (f32::from(tip.x - from.x), f32::from(tip.y - from.y));
    let length = dx.hypot(dy);
    if length < 0.001 {
        return;
    }
    let (ux, uy) = (dx / length, dy / length);
    // The two barbs sit back along the line and out to either side of it.
    let back = point(px(-ux * size), px(-uy * size));
    let across = point(px(-uy * size * 0.45), px(ux * size * 0.45));

    let mut builder = PathBuilder::fill();
    builder.add_polygon(&[tip, tip + back + across, tip + back - across], true);
    if let Ok(path) = builder.build() {
        window.paint_path(path, colour);
    }
}

/// Text, in the element's own colour and at its own size.
///
/// Each line is shaped and painted separately — the alignment is resolved against the element's
/// box by measuring, which is also what makes a centred label sit where Excalidraw put it. Rotation
/// is not applied; see the module header.
#[allow(clippy::too_many_arguments)]
fn label(
    element: &Element,
    text: &str,
    font_size: f32,
    family: FontFamily,
    align: TextAlign,
    view: &View,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(colour) = element.stroke else {
        return;
    };
    let size = view.len(font_size);
    if size < 3.0 || text.is_empty() {
        // Below a few pixels a glyph is noise. Nothing is drawn rather than a smear.
        return;
    }
    let colour = tint(colour, element.opacity);
    let line_height = size * 1.25;
    let top_left = view.at(element.x, element.y);
    let width = view.len(element.width);

    for (row, line) in text.split('\n').enumerate() {
        if line.is_empty() {
            continue;
        }
        let shaped = shape(line, size, family, colour, window);
        let shift = match align {
            TextAlign::Left => 0.0,
            TextAlign::Center => (width - f32::from(shaped.width())) / 2.0,
            TextAlign::Right => width - f32::from(shaped.width()),
        };
        let origin = top_left + point(px(shift), px(line_height * row as f32));
        let _ = shaped.paint(
            origin,
            px(line_height),
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        );
    }
}

/// One shaped line, in the interface's own font unless the element asked for the monospaced one.
///
/// Excalidraw's `hand` family has no equivalent here, and asking for one would be the same mistake
/// as sketching the strokes: the file's hand-drawn intent renders as clean vector throughout.
fn shape(
    text: &str,
    size: f32,
    family: FontFamily,
    colour: Rgba,
    window: &mut Window,
) -> gpui::ShapedLine {
    let mut font = window.text_style().font();
    if family == FontFamily::Code {
        font.family = theme::MONO_FONT.into();
    }
    let run = TextRun {
        len: text.len(),
        font,
        color: colour.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text.to_string().into(), px(size), &[run], None)
}

/// What an embedded image draws as. The bytes are in the scene; decoding them is not this crate's
/// job today, so the reader gets the box the image occupies rather than a hole in the layout.
fn placeholder(element: &Element, view: &View, window: &mut Window) {
    let colour = theme::fade(theme::text_faint(), element.opacity.min(0.7));
    let mapped: Vec<Point<Pixels>> = corner_points(element)
        .into_iter()
        .map(|(x, y)| place(element, view, x, y))
        .collect();

    let mut builder = PathBuilder::stroke(px(view.stroke_width(1.0))).dash_array(&[px(4.), px(3.)]);
    builder.add_polygon(&mapped, true);
    if let Ok(path) = builder.build() {
        window.paint_path(path, colour);
    }
    // A diagonal, so an empty box reads as a picture that is not there rather than as a shape.
    let mut cross = PathBuilder::stroke(px(view.stroke_width(1.0)));
    cross.move_to(mapped[0]);
    cross.line_to(mapped[2]);
    if let Ok(path) = cross.build() {
        window.paint_path(path, colour);
    }
}
