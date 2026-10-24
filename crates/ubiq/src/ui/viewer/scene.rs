//! A drawing, painted — Excalidraw and draw.io through one painter.
//!
//! Both formats are **data with a closed vocabulary**, not languages, so there is nothing here to
//! delegate to: `crates/ubiq-draw` turns either file into one format-agnostic
//! [`ubiq_draw::Drawing`] — a group tree in paint order, shapes with their own ink, text boxes,
//! connectors — and this module walks it with `canvas()` and `PathBuilder`. **A drawing read from a
//! file is read-only.** The one scene that is edited is the capture editor's, which Ubiq authored
//! in this session and which reaches this painter through `live_with_overlay` — the tools live in
//! `ui/viewer/image_edit.rs`, and nothing here edits anything.
//!
//! The drawing is fitted to the panel with a margin and its aspect ratio preserved. The wheel zooms
//! about the pointer, a drag pans, a double-click restores the fit. A scene in a Markdown fence is
//! drawn at its own size instead, because a fence is a block in a document and not a panel.
//!
//! ## The limits
//!
//! - **Excalidraw's hand-drawn `roughness` renders as clean vector**, and the `hand` font family
//!   falls back to the interface's own. Hachure and cross-hatch fills render solid.
//! - **Text is never rotated and never wrapped**, because a glyph run is painted, not transformed
//!   or flowed: each `\n` line is shaped once and aligned in its box. Every other shape honours
//!   its rotation.
//! - **An embedded image paints its bytes at its box**, under the vector layer, where the mime type
//!   names a format the platform decodes. An image with no bytes, or one rotated, draws as a
//!   placeholder box instead.
//! - **A shape the converter could not draw is a dashed placeholder with its label**, never a hole.
//!
//! ## Colour
//!
//! `_docs/tech/ui-and-design.md` forbids a literal colour outside `theme.rs`, and **this module
//! breaks no part of it.** A drawing's own strokes and fills are read out of the file: they are
//! data, the way a photograph's pixels are, and passing them to `paint_path` is not a design
//! decision. Every colour *this module chooses* — the placeholder a missing image or an
//! unsupported shape draws as, the text of a failure note — is a theme token. The ground behind a
//! drawing that names none is the format's own white paper ([`Rgba8::DEFAULT_BACKGROUND`]), which
//! is data from the format, the way a stroke colour is.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use gpui::{
    AnyElement, App, Bounds, Context, Image, ImageFormat, ImageSource, IntoElement, ParentElement,
    PathBuilder, Pixels, Point, Rgba, Styled, TextRun, Window, canvas, div, fill, img, point, px,
    size,
};
use ubiq_draw::{
    ArrowHead, Drawing, EmbeddedImage, FontFamily, Geometry, Rect, Shape, StrokeStyle, TextAlign,
    TextBox, VAlign,
};

use crate::app::AppState;
use crate::state::scene::{Rgba8, Scene, SceneError};
use crate::state::viewport::{self, Camera, Content};
use crate::theme;

/// How many segments an ellipse is drawn with. Enough that the seams are invisible at any zoom the
/// fit produces, and cheap enough that a wireframe full of them costs nothing.
const ELLIPSE_STEPS: usize = 64;

/// The largest on-screen font size a label is drawn at; past it the camera is zoomed into one
/// glyph and shaping it would cost more than it shows.
const MAX_LABEL_PX: f32 = 2000.0;

/// How many converted drawings the window keeps before it starts over.
const CACHE_SIZE: usize = 32;

/// What a source converted to, kept across frames so a file is parsed once per change.
#[derive(Clone)]
enum Converted {
    Drawing(Arc<Drawing>),
    Empty,
    Failed(Option<SceneError>),
}

thread_local! {
    /// Converted drawings keyed by format and the source's hash. The window is single-threaded,
    /// and a frame that changed nothing must not parse again; the map is cleared when it fills,
    /// which costs one parse per open drawing.
    static CONVERTED: RefCell<HashMap<(bool, u64), Converted>> = RefCell::new(HashMap::new());
}

/// `convert(source)` once per distinct source.
fn converted(drawio: bool, source: &str, convert: impl FnOnce() -> Converted) -> Converted {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    let key = (drawio, hasher.finish());
    if let Some(hit) = CONVERTED.with_borrow(|cache| cache.get(&key).cloned()) {
        return hit;
    }
    let value = convert();
    CONVERTED.with_borrow_mut(|cache| {
        if cache.len() >= CACHE_SIZE {
            cache.clear();
        }
        cache.insert(key, value.clone());
    });
    value
}

/// An Excalidraw scene in a Markdown fence.
pub fn render(source: &str) -> AnyElement {
    match parsed(source) {
        Ok(drawing) => draw_static(drawing, source),
        Err(note) => note,
    }
}

/// An Excalidraw scene in a panel the user can pan and zoom.
pub fn live(app: &AppState, key: &str, source: &str, cx: &mut Context<AppState>) -> AnyElement {
    match parsed(source) {
        Ok(drawing) => live_drawing(app, key, drawing, div().into_any_element(), cx),
        Err(note) => note,
    }
}

/// A draw.io diagram in a panel — its first page, through the same painter.
///
/// A file the converter cannot read at all falls back to the SVG the draw.io editor last exported
/// ([`super::diagram::exported`]), so a diagram that ever drew keeps drawing.
pub fn drawio_live(
    app: &AppState,
    key: &str,
    source: &str,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let result = converted(true, source, || {
        match ubiq_draw::drawio::parse(source.as_bytes()) {
            Ok(drawing) if drawing.is_empty() => Converted::Empty,
            Ok(drawing) => Converted::Drawing(Arc::new(drawing)),
            Err(_) => Converted::Failed(None),
        }
    });
    match result {
        Converted::Empty => super::note("this diagram is empty", theme::text_faint()),
        Converted::Drawing(drawing) => {
            live_drawing(app, key, drawing, div().into_any_element(), cx)
        }
        Converted::Failed(_) => super::diagram::exported(app, key, source, cx),
    }
}

fn parsed(source: &str) -> Result<Arc<Drawing>, AnyElement> {
    let result = converted(false, source, || match Scene::parse(source.as_bytes()) {
        Ok(scene) if scene.elements.is_empty() => Converted::Empty,
        Ok(scene) => Converted::Drawing(Arc::new(scene.to_drawing())),
        Err(why) => Converted::Failed(Some(why)),
    });
    match result {
        Converted::Drawing(drawing) => Ok(drawing),
        Converted::Empty => Err(super::note("this scene is empty", theme::text_faint())),
        // Every failure says why in the reader's own words, including the compressed case — which
        // is a scene that exists and cannot be unpacked, not a file that went wrong.
        Converted::Failed(Some(why @ SceneError::Compressed)) => {
            Err(super::note(why.to_string(), theme::text_muted()))
        }
        Converted::Failed(why) => Err(super::note(
            why.map(|why| why.to_string()).unwrap_or_default(),
            theme::danger(),
        )),
    }
}

fn content_of(drawing: &Drawing) -> Content {
    Content {
        min_x: drawing.bounds.min_x,
        min_y: drawing.bounds.min_y,
        width: drawing.bounds.width(),
        height: drawing.bounds.height(),
    }
}

/// The file's canvas colour, or the format's white paper when it named none.
fn ground_of(drawing: &Drawing) -> Rgba {
    tint(drawing.background.unwrap_or(Rgba8::DEFAULT_BACKGROUND), 1.0)
}

/// A scene in a Markdown fence: drawn at its own size, no camera of its own, because a fence is a
/// block in a document and the document is what scrolls. A scene still wider than the reading
/// column once scaled down to it (T-185) scrolls inside `super::diagram_frame` (T-126) rather than
/// spilling past the viewport.
///
/// **No zoom button here** (T-185): unlike a Mermaid fence, which resolves to one `Arc<Image>` the
/// modal and the Copy button can both hand off whole, a scene is vector shapes painted straight
/// into the panel by [`paint`] — there is no single picture to raise or to copy, only the same
/// live camera [`live_drawing`] already gives the panel view.
fn draw_static(drawing: Arc<Drawing>, key: &str) -> AnyElement {
    let content = content_of(&drawing);
    let natural_w = content.width.max(1.0) + viewport::MARGIN * 2.0;
    let natural_h = content.height.max(1.0) + viewport::MARGIN * 2.0;
    let (panel_w, panel_h) =
        super::diagram::scale_to_measure(natural_w, natural_h, super::diagram::current_measure());
    let camera = viewport::Viewport::default().camera(content, panel_w, panel_h);
    let ground = ground_of(&drawing);
    let pictures = images_static(&drawing, camera);

    let picture = div()
        .flex_none()
        .relative()
        .w(px(panel_w))
        .h(px(panel_h))
        .bg(ground)
        // The pictures paint under the vector layer, because an embedded image is a ground the
        // rest of the scene is drawn on — which is also the order the flatten writes.
        .children(pictures)
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let view = View::from_camera(camera, bounds.origin);
                    for shape in drawing.shapes() {
                        paint(shape, &drawing.images, &view, window, cx);
                    }
                },
            )
            .absolute()
            .inset_0()
            .w(px(panel_w))
            .h(px(panel_h)),
        );
    super::diagram_frame(key, picture)
}

/// The image editor's scene live on its camera, with one element above the picture: its tool
/// layer. The overlay sits over the viewport's own hit layer, so a tool drag lands on the tool and
/// anything it declines reaches pan and zoom underneath.
pub(crate) fn live_with_overlay(
    app: &AppState,
    key: &str,
    scene: &Scene,
    overlay: AnyElement,
    cx: &mut Context<AppState>,
) -> AnyElement {
    // The scene under edit changes between frames, so it is converted each frame — no parse.
    live_drawing(app, key, Arc::new(scene.to_drawing()), overlay, cx)
}

fn live_drawing(
    app: &AppState,
    key: &str,
    drawing: Arc<Drawing>,
    overlay: AnyElement,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let content = content_of(&drawing);
    let camera_at = app.viewport(key);
    // The overlay positions follow the stored panel, as the diagram viewer does; the canvas
    // re-fits to its live bounds each paint, and the two agree once the panel is measured.
    let overlay_cam = camera_at.camera(content, camera_at.panel_w, camera_at.panel_h);
    let pictures = images_static(&drawing, overlay_cam);
    super::viewport::surface(
        app,
        key,
        ground_of(&drawing),
        content,
        div()
            .absolute()
            .inset_0()
            .size_full()
            .relative()
            // The pictures first: an embedded image is the ground the annotations are drawn on,
            // in the panel exactly as in the flatten.
            .children(pictures)
            .child(
                canvas(|_, _, _| {}, {
                    move |bounds, _, window, cx| {
                        let camera = camera_at.camera(
                            content,
                            f32::from(bounds.size.width),
                            f32::from(bounds.size.height),
                        );
                        let view = View::from_camera(camera, bounds.origin);
                        for shape in drawing.shapes() {
                            paint(shape, &drawing.images, &view, window, cx);
                        }
                    }
                })
                .absolute()
                .inset_0()
                .size_full(),
            ),
        overlay,
        cx,
    )
}

// ------------------------------------------------------------------------------------------- //
// The transform
// ------------------------------------------------------------------------------------------- //

/// Drawing coordinates to window coordinates: one uniform scale and one offset, so the aspect
/// ratio survives and a diagram is never stretched to fit.
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

    /// A drawing length in window pixels.
    fn len(&self, value: f32) -> f32 {
        value * self.scale
    }

    /// A stroke never thins below a hairline, or a scaled-down diagram loses its outlines.
    fn stroke_width(&self, width: f32) -> f32 {
        self.len(width).max(0.75)
    }
}

/// One point of a shape, rotated about the centre of its frame and then placed.
fn place(shape: &Shape, view: &View, x: f32, y: f32) -> Point<Pixels> {
    if shape.rotation == 0.0 {
        return view.at(x, y);
    }
    let c = shape.frame.centre();
    let (sin, cos) = shape.rotation.sin_cos();
    let (dx, dy) = (x - c.x, y - c.y);
    view.at(c.x + dx * cos - dy * sin, c.y + dx * sin + dy * cos)
}

fn placed(shape: &Shape, view: &View, points: &[(f32, f32)]) -> Vec<Point<Pixels>> {
    points
        .iter()
        .map(|&(x, y)| place(shape, view, x, y))
        .collect()
}

// ------------------------------------------------------------------------------------------- //
// Colour and stroke
// ------------------------------------------------------------------------------------------- //

/// A colour the file named, at the shape's opacity. Data from the drawing, not a token — see the
/// module header.
fn tint(colour: Rgba8, opacity: f32) -> Rgba {
    Rgba {
        r: f32::from(colour.r) / 255.0,
        g: f32::from(colour.g) / 255.0,
        b: f32::from(colour.b) / 255.0,
        a: f32::from(colour.a) / 255.0 * opacity,
    }
}

/// A stroke builder carrying the shape's width and dash pattern, both scaled with the view so a
/// dashed line stays dashed at any zoom.
fn pen(view: &View, width: f32, style: StrokeStyle) -> PathBuilder {
    let builder = PathBuilder::stroke(px(view.stroke_width(width)));
    match style {
        StrokeStyle::Solid => builder,
        StrokeStyle::Dashed => {
            builder.dash_array(&[px(view.len(10.0).max(2.0)), px(view.len(6.0).max(2.0))])
        }
        StrokeStyle::Dotted => {
            builder.dash_array(&[px(view.stroke_width(2.0)), px(view.len(6.0).max(2.0))])
        }
    }
}

// ------------------------------------------------------------------------------------------- //
// The shapes
// ------------------------------------------------------------------------------------------- //

fn paint(
    shape: &Shape,
    images: &HashMap<String, EmbeddedImage>,
    view: &View,
    window: &mut Window,
    cx: &mut App,
) {
    match &shape.geometry {
        Geometry::Rect => rectangle(shape, 0.0, view, window),
        Geometry::RoundedRect { radius } => rectangle(shape, *radius, view, window),
        Geometry::Ellipse => outline(shape, ellipse_points(shape.frame), true, view, window),
        Geometry::Diamond => outline(shape, diamond_points(shape.frame), true, view, window),
        Geometry::Polyline { points, closed } => {
            let points: Vec<(f32, f32)> = points.iter().map(|p| (p.x, p.y)).collect();
            outline(shape, points, *closed, view, window);
        }
        Geometry::Freehand { points } => {
            let points: Vec<(f32, f32)> = points.iter().map(|p| (p.x, p.y)).collect();
            stroke_polygon(shape, &points, false, view, window);
        }
        Geometry::Connector(connector) => {
            let points: Vec<(f32, f32)> = connector.points.iter().map(|p| (p.x, p.y)).collect();
            line(shape, &points, connector.start, connector.end, view, window);
        }
        Geometry::Text => {}
        Geometry::Image { image } => {
            // A resolvable image paints as an overlay child, not on the canvas; the rest fall
            // through to the placeholder so a missing picture never leaves a hole.
            let resolved = image
                .as_deref()
                .and_then(|id| resolved_image(images, id, shape.rotation));
            if resolved.is_none() {
                placeholder(shape, true, view, window);
            }
        }
        Geometry::Placeholder => placeholder(shape, false, view, window),
    }
    if let Some(text) = &shape.text {
        label(text, shape.style.opacity, view, window, cx);
    }
}

/// A rectangle. Unrotated and square-cornered, it is a quad — the cheapest thing GPUI paints, and
/// what most of a wireframe is made of. Anything else goes through a path.
fn rectangle(shape: &Shape, radius: f32, view: &View, window: &mut Window) {
    let r = shape.frame;
    let radius = radius.min(r.w.abs() / 2.0).min(r.h.abs() / 2.0).max(0.0);

    if shape.rotation == 0.0 && view.len(radius) <= 0.5 {
        if let Some(colour) = shape.style.fill {
            let top_left = view.at(r.x, r.y);
            let extent = size(px(view.len(r.w)), px(view.len(r.h)));
            window.paint_quad(fill(
                Bounds::new(top_left, extent),
                tint(colour, shape.style.opacity),
            ));
        }
        stroke_polygon(shape, &corner_points(r), true, view, window);
        return;
    }

    if view.len(radius) <= 0.5 {
        outline(shape, corner_points(r), true, view, window);
        return;
    }

    // Rounded, or rotated, or both: one path either way. The corners are quadratic curves with the
    // sharp corner as their control point, which is what an SVG `rx` draws.
    let round = |builder: &mut PathBuilder| {
        let (x, y, w, h) = (r.x, r.y, r.w, r.h);
        let p = |bx: f32, by: f32| place(shape, view, bx, by);
        let rr = radius;
        builder.move_to(p(x + rr, y));
        builder.line_to(p(x + w - rr, y));
        builder.curve_to(p(x + w, y + rr), p(x + w, y));
        builder.line_to(p(x + w, y + h - rr));
        builder.curve_to(p(x + w - rr, y + h), p(x + w, y + h));
        builder.line_to(p(x + rr, y + h));
        builder.curve_to(p(x, y + h - rr), p(x, y + h));
        builder.line_to(p(x, y + rr));
        builder.curve_to(p(x + rr, y), p(x, y));
        builder.close();
    };

    if let Some(colour) = shape.style.fill {
        let mut builder = PathBuilder::fill();
        round(&mut builder);
        if let Ok(path) = builder.build() {
            window.paint_path(path, tint(colour, shape.style.opacity));
        }
    }
    if let Some(colour) = shape.style.stroke {
        let mut builder = pen(view, shape.style.stroke_width, shape.style.stroke_style);
        round(&mut builder);
        if let Ok(path) = builder.build() {
            window.paint_path(path, tint(colour, shape.style.opacity));
        }
    }
}

/// A shape given as drawing-space points: filled when closed, then stroked.
fn outline(shape: &Shape, points: Vec<(f32, f32)>, closed: bool, view: &View, window: &mut Window) {
    if closed
        && points.len() > 2
        && let Some(colour) = shape.style.fill
    {
        let mut builder = PathBuilder::fill();
        builder.add_polygon(&placed(shape, view, &points), true);
        if let Ok(path) = builder.build() {
            window.paint_path(path, tint(colour, shape.style.opacity));
        }
    }
    stroke_polygon(shape, &points, closed, view, window);
}

fn stroke_polygon(
    shape: &Shape,
    points: &[(f32, f32)],
    closed: bool,
    view: &View,
    window: &mut Window,
) {
    let Some(colour) = shape.style.stroke else {
        return;
    };
    if points.len() < 2 || shape.style.stroke_width <= 0.0 {
        return;
    }
    let mut builder = pen(view, shape.style.stroke_width, shape.style.stroke_style);
    builder.add_polygon(&placed(shape, view, points), closed);
    if let Ok(path) = builder.build() {
        window.paint_path(path, tint(colour, shape.style.opacity));
    }
}

fn corner_points(r: Rect) -> Vec<(f32, f32)> {
    vec![
        (r.x, r.y),
        (r.x + r.w, r.y),
        (r.x + r.w, r.y + r.h),
        (r.x, r.y + r.h),
    ]
}

fn diamond_points(r: Rect) -> Vec<(f32, f32)> {
    vec![
        (r.x + r.w / 2.0, r.y),
        (r.x + r.w, r.y + r.h / 2.0),
        (r.x + r.w / 2.0, r.y + r.h),
        (r.x, r.y + r.h / 2.0),
    ]
}

fn ellipse_points(r: Rect) -> Vec<(f32, f32)> {
    let (rx, ry) = (r.w / 2.0, r.h / 2.0);
    let (cx, cy) = (r.x + rx, r.y + ry);
    (0..ELLIPSE_STEPS)
        .map(|step| {
            let theta = std::f32::consts::TAU * step as f32 / ELLIPSE_STEPS as f32;
            (cx + rx * theta.cos(), cy + ry * theta.sin())
        })
        .collect()
}

/// A connector: a polyline through absolute points, with a head on either end that asked for
/// one.
fn line(
    shape: &Shape,
    points: &[(f32, f32)],
    start: ArrowHead,
    end: ArrowHead,
    view: &View,
    window: &mut Window,
) {
    let Some(colour) = shape.style.stroke else {
        return;
    };
    let mapped = placed(shape, view, points);
    if mapped.len() < 2 {
        return;
    }
    let paint_colour = tint(colour, shape.style.opacity);

    let mut builder = pen(view, shape.style.stroke_width, shape.style.stroke_style);
    builder.add_polygon(&mapped, false);
    if let Ok(path) = builder.build() {
        window.paint_path(path, paint_colour);
    }

    // Heads follow the stroke's width, as the reference renderer's marker does.
    let head = view.len(9.0 + shape.style.stroke_width * 1.5).max(4.0);
    let width = view.stroke_width(shape.style.stroke_width);
    let n = mapped.len();
    arrowhead(
        end,
        mapped[n - 1],
        mapped[n - 2],
        head,
        width,
        paint_colour,
        window,
    );
    arrowhead(
        start,
        mapped[0],
        mapped[1],
        head,
        width,
        paint_colour,
        window,
    );
}

fn arrowhead(
    kind: ArrowHead,
    tip: Point<Pixels>,
    from: Point<Pixels>,
    size: f32,
    width: f32,
    colour: Rgba,
    window: &mut Window,
) {
    if kind == ArrowHead::None {
        return;
    }
    let (dx, dy) = (f32::from(tip.x - from.x), f32::from(tip.y - from.y));
    let length = dx.hypot(dy);
    if length < 0.001 {
        return;
    }
    let (ux, uy) = (dx / length, dy / length);
    // The two barbs sit back along the line and out to either side of it.
    let back = point(px(-ux * size), px(-uy * size));
    let across = point(px(-uy * size * 0.45), px(ux * size * 0.45));
    let barbs = [tip + back + across, tip, tip + back - across];

    let mut builder = match kind {
        ArrowHead::Open => PathBuilder::stroke(px(width)),
        _ => PathBuilder::fill(),
    };
    builder.add_polygon(&barbs, kind != ArrowHead::Open);
    if let Ok(path) = builder.build() {
        window.paint_path(path, colour);
    }
}

/// A text box, in its own colour and size: each line shaped and aligned in the box, the block
/// placed vertically by its `valign`. Never rotated; see the module header.
fn label(text: &TextBox, opacity: f32, view: &View, window: &mut Window, cx: &mut App) {
    let Some(colour) = text.colour else {
        return;
    };
    let size = view.len(text.font_size);
    if !(3.0..=MAX_LABEL_PX).contains(&size) || text.text.is_empty() {
        // Below a few pixels a glyph is noise, and above a couple of thousand it is one letter
        // filling the panel. Nothing is drawn rather than a smear or a huge shaping job.
        return;
    }
    let colour = tint(colour, opacity);
    let line_height = size * 1.25;
    let lines: Vec<&str> = text.text.split('\n').collect();
    let block = line_height * lines.len() as f32;
    let (x0, x1) = (
        text.rect.x.min(text.rect.x + text.rect.w),
        text.rect.x.max(text.rect.x + text.rect.w),
    );
    let (y0, y1) = (
        text.rect.y.min(text.rect.y + text.rect.h),
        text.rect.y.max(text.rect.y + text.rect.h),
    );
    let width = view.len(x1 - x0);
    let height = view.len(y1 - y0);
    let drop = match text.valign {
        VAlign::Top => 0.0,
        VAlign::Middle => (height - block) / 2.0,
        VAlign::Bottom => height - block,
    };
    let top_left = view.at(x0, y0) + point(px(0.), px(drop));

    for (row, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let shaped = shape_line(line, size, text.family, colour, window);
        let shift = match text.align {
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

/// One shaped line, in the interface's own font unless the text asked for the monospaced one.
///
/// Excalidraw's `hand` family has no equivalent here, and asking for one would be the same mistake
/// as sketching the strokes: the file's hand-drawn intent renders as clean vector throughout.
fn shape_line(
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

/// The platform format a `data:` URI's mime type names. `None` is "not decodable here", and
/// the image keeps its placeholder. Mirrors [`super::image`]'s extension mapping.
fn image_format(mime: &str) -> Option<ImageFormat> {
    match mime.trim().to_ascii_lowercase().as_str() {
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" | "image/jpg" => Some(ImageFormat::Jpeg),
        "image/gif" => Some(ImageFormat::Gif),
        "image/webp" => Some(ImageFormat::Webp),
        "image/svg+xml" | "image/svg" => Some(ImageFormat::Svg),
        "image/bmp" | "image/x-bmp" | "image/x-ms-bmp" => Some(ImageFormat::Bmp),
        "image/tiff" | "image/x-tiff" => Some(ImageFormat::Tiff),
        "image/x-icon" | "image/vnd.microsoft.icon" => Some(ImageFormat::Ico),
        _ => None,
    }
}

/// A paintable image: bytes the file carried, in a decodable format, unrotated. Rotation has no
/// `img` equivalent, so a rotated image keeps its placeholder rather than painting unrotated.
fn resolved_image<'a>(
    images: &'a HashMap<String, EmbeddedImage>,
    id: &str,
    rotation: f32,
) -> Option<(ImageFormat, &'a [u8])> {
    if rotation != 0.0 {
        return None;
    }
    let file = images.get(id)?;
    Some((image_format(&file.mime)?, &file.bytes))
}

/// One positioned `img` per paintable image, in the camera's coordinates — the same absolute
/// placement [`super::diagram`] uses for its picture.
fn images_static(drawing: &Drawing, camera: Camera) -> Vec<AnyElement> {
    drawing
        .shapes()
        .into_iter()
        .filter_map(|shape| {
            let Geometry::Image { image: Some(id) } = &shape.geometry else {
                return None;
            };
            let (format, bytes) = resolved_image(&drawing.images, id, shape.rotation)?;
            let r = shape.frame;
            let (x, w) = if r.w >= 0.0 {
                (r.x, r.w)
            } else {
                (r.x + r.w, -r.w)
            };
            let (y, h) = if r.h >= 0.0 {
                (r.y, r.h)
            } else {
                (r.y + r.h, -r.h)
            };
            let picture = Arc::new(Image::from_bytes(format, bytes.to_vec()));
            Some(
                img(ImageSource::Image(picture))
                    .absolute()
                    .left(px(camera.offset_x + x * camera.scale))
                    .top(px(camera.offset_y + y * camera.scale))
                    .w(px(w * camera.scale))
                    .h(px(h * camera.scale))
                    .opacity(shape.style.opacity)
                    .into_any_element(),
            )
        })
        .collect()
}

/// What a missing image or an unsupported shape draws as: a dashed box in a theme colour, with a
/// diagonal for a picture that is not there. An unsupported shape's label is drawn by [`paint`].
fn placeholder(shape: &Shape, cross: bool, view: &View, window: &mut Window) {
    let colour = theme::fade(theme::text_faint(), shape.style.opacity.min(0.7));
    let mapped = placed(shape, view, &corner_points(shape.frame));

    let mut builder = PathBuilder::stroke(px(view.stroke_width(1.0))).dash_array(&[px(4.), px(3.)]);
    builder.add_polygon(&mapped, true);
    if let Ok(path) = builder.build() {
        window.paint_path(path, colour);
    }
    if cross {
        let mut diagonal = PathBuilder::stroke(px(view.stroke_width(1.0)));
        diagonal.move_to(mapped[0]);
        diagonal.line_to(mapped[2]);
        if let Ok(path) = diagonal.build() {
            window.paint_path(path, colour);
        }
    }
}
