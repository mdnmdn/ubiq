//! An annotated capture: the buffer behind an image tab the session authored.
//!
//! The base PNG is held whole and never mutated. Every tool appends, moves or removes an
//! [`Element`] above it, and crop only narrows [`Scene::bounds`]. Undo pops the edit stack, so
//! no pixel snapshot is ever kept — a freehand stroke costs its points, not a second bitmap.
//!
//! Flattening (save, copy-region) decodes the base, overlays the annotations at scene scale
//! cropped to the bounds, and re-encodes PNG. Nothing else in the interface decodes: display
//! stays GPUI's, and this module only measures (base dimensions) and flattens.
//!
//! Only untitled capture tabs ever hold one of these — see [`crate::state::editor::OpenFile`] —
//! so a PNG from the explorer and an `.excalidraw` document both stay read-only.

use std::collections::HashMap;

use ubiq_proto::files::FileVersion;

use super::scene::{
    Bounds, Element, ElementKind, EmbeddedFile, FontFamily, Rgba8, Scene, StrokeStyle, TextAlign,
};

/// The file id the captured PNG travels under in the scene's `files` map — which is precisely
/// what that map is for.
const BASE_ID: &str = "capture";

/// The toolbar's vocabulary: a subset of [`ElementKind`], and nothing new invented.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ImageTool {
    #[default]
    Select,
    Crop,
    Rectangle,
    Ellipse,
    Arrow,
    Freehand,
    Text,
    Copy,
}

impl ImageTool {
    pub fn label(self) -> &'static str {
        match self {
            ImageTool::Select => "Select",
            ImageTool::Crop => "Crop",
            ImageTool::Rectangle => "Rect",
            ImageTool::Ellipse => "Ellipse",
            ImageTool::Arrow => "Arrow",
            ImageTool::Freehand => "Draw",
            ImageTool::Text => "Text",
            ImageTool::Copy => "Copy",
        }
    }

    pub fn all() -> [ImageTool; 8] {
        [
            ImageTool::Select,
            ImageTool::Crop,
            ImageTool::Rectangle,
            ImageTool::Ellipse,
            ImageTool::Arrow,
            ImageTool::Freehand,
            ImageTool::Text,
            ImageTool::Copy,
        ]
    }
}

/// One undoable scene edit. Element snapshots, never pixels: even `Replace` is one shape.
#[derive(Clone, PartialEq, Debug)]
pub enum EditOp {
    Add {
        element: Element,
    },
    Remove {
        element: Element,
        index: usize,
    },
    Replace {
        before: Element,
        after: Element,
        index: usize,
    },
    Crop {
        from: Bounds,
        to: Bounds,
    },
}

/// A drag in progress. Transient UI state that travels with the tab because nothing else is
/// per-tab; never persisted (view prefs keep paths, not bodies) and never on the undo stack —
/// the commit is.
#[derive(Clone, PartialEq, Debug)]
pub enum Pending {
    /// A dragged rectangle: a shape, the crop, or the copy region.
    Shape {
        kind: ShapeKind,
        start: (f32, f32),
        cur: (f32, f32),
    },
    /// A freehand stroke being collected.
    Draw { points: Vec<(f32, f32)> },
    /// A selected element following the pointer.
    Move {
        id: String,
        orig: (f32, f32),
        grab: (f32, f32),
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShapeKind {
    Rect,
    Ellipse,
    Arrow,
    Crop,
    Copy,
}

/// The stroke colours the toolbar cycles. Data, the way a scene's own colours are.
pub const STROKES: [Rgba8; 6] = [
    Rgba8::opaque(0x1e, 0x1e, 0x1e),
    Rgba8::opaque(0xe0, 0x31, 0x31),
    Rgba8::opaque(0x25, 0x63, 0xeb),
    Rgba8::opaque(0x16, 0xa3, 0x4a),
    Rgba8::opaque(0xea, 0x88, 0x0c),
    Rgba8::opaque(0xff, 0xff, 0xff),
];

/// The widths the toolbar cycles.
pub const WIDTHS: [f32; 3] = [2.0, 4.0, 8.0];

pub struct ImageEdit {
    /// The captured PNG, untouched for the life of the buffer.
    pub base: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Elements `[0]` is the base image; every annotation is appended after it.
    pub scene: Scene,
    /// The pre-crop extent. A crop narrows `scene.bounds`; re-cropping measures against this,
    /// so an earlier crop never eats the picture.
    pub full: Bounds,
    pub undo: Vec<EditOp>,
    pub redo: Vec<EditOp>,
    pub tool: ImageTool,
    pub selected: Option<String>,
    pub stroke: Option<Rgba8>,
    pub fill: Option<Rgba8>,
    pub stroke_width: f32,
    pub version: Option<FileVersion>,
    pub pending: Option<Pending>,
    /// Where the next typed text lands, set by the Text tool's click until the dialog answers.
    pub text_at: Option<(f32, f32)>,
    next_id: u64,
}

impl ImageEdit {
    /// Build the editable scene over captured PNG bytes: the bytes under [`BASE_ID`], one
    /// [`ElementKind::Image`] over them at pixel size, bounds to match. `None` is bytes with no
    /// decodable picture in them, and the caller keeps those read-only.
    pub fn new(base: &[u8], version: Option<FileVersion>) -> Option<Self> {
        let (width, height) = image::load_from_memory(base)
            .ok()
            .map(|decoded| (decoded.width().max(1), decoded.height().max(1)))?;
        let bounds = Bounds {
            min_x: 0.0,
            min_y: 0.0,
            max_x: width as f32,
            max_y: height as f32,
        };
        let mut files = HashMap::new();
        files.insert(
            BASE_ID.to_string(),
            EmbeddedFile {
                mime: "image/png".to_string(),
                bytes: base.to_vec(),
            },
        );
        Some(Self {
            base: base.to_vec(),
            width,
            height,
            scene: Scene {
                elements: vec![Element {
                    id: BASE_ID.to_string(),
                    kind: ElementKind::Image {
                        file_id: BASE_ID.to_string(),
                    },
                    x: 0.0,
                    y: 0.0,
                    width: width as f32,
                    height: height as f32,
                    angle: 0.0,
                    stroke: None,
                    fill: None,
                    stroke_width: 0.0,
                    stroke_style: StrokeStyle::Solid,
                    opacity: 1.0,
                }],
                background: Some(Rgba8::DEFAULT_BACKGROUND),
                files,
                bounds,
            },
            full: bounds,
            undo: Vec::new(),
            redo: Vec::new(),
            tool: ImageTool::default(),
            selected: None,
            stroke: Some(STROKES[0]),
            fill: None,
            stroke_width: WIDTHS[0],
            version,
            pending: None,
            text_at: None,
            next_id: 1,
        })
    }

    fn id(&mut self) -> String {
        let id = format!("el-{}", self.next_id);
        self.next_id += 1;
        id
    }

    fn styled(&self, id: String, kind: ElementKind, x: f32, y: f32, w: f32, h: f32) -> Element {
        Element {
            id,
            kind,
            x,
            y,
            width: w,
            height: h,
            angle: 0.0,
            stroke: self.stroke,
            fill: self.fill,
            stroke_width: self.stroke_width,
            stroke_style: StrokeStyle::Solid,
            opacity: 1.0,
        }
    }

    /// Append one element, in paint order. The stack is what undo pops.
    pub fn push(&mut self, element: Element) {
        self.scene.elements.push(element.clone());
        self.scene
            .elements
            .sort_by_key(|element| element.kind.paint_rank());
        self.undo.push(EditOp::Add { element });
        self.redo.clear();
    }

    /// The topmost annotatable element under a scene point. The base image is never an answer:
    /// it is not moved, recoloured or deleted.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<String> {
        self.scene
            .elements
            .iter()
            .rev()
            .filter(|element| !matches!(element.kind, ElementKind::Image { .. }))
            .find(|element| contains(element, x, y))
            .map(|element| element.id.clone())
    }

    fn index_of(&self, id: &str) -> Option<usize> {
        self.scene.elements.iter().position(|el| el.id == id)
    }

    /// Delete the selection. `false` is nothing selected, or the base somehow named.
    pub fn delete_selected(&mut self) -> bool {
        let Some(id) = self.selected.clone() else {
            return false;
        };
        let Some(index) = self.index_of(&id) else {
            return false;
        };
        if matches!(self.scene.elements[index].kind, ElementKind::Image { .. }) {
            return false;
        }
        let element = self.scene.elements.remove(index);
        self.undo.push(EditOp::Remove { element, index });
        self.redo.clear();
        self.selected = None;
        true
    }

    /// Move a live drag: the element follows the pointer, no op yet — the mouse-up commits.
    pub fn live_move(&mut self, id: &str, dx: f32, dy: f32, orig: (f32, f32)) {
        if let Some(index) = self.index_of(id) {
            self.scene.elements[index].x = orig.0 + dx;
            self.scene.elements[index].y = orig.1 + dy;
        }
    }

    /// Commit a finished drag as one undoable move.
    pub fn commit_move(&mut self, id: &str, before: (f32, f32)) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        let after = (self.scene.elements[index].x, self.scene.elements[index].y);
        if (after.0 - before.0).abs() < 0.5 && (after.1 - before.1).abs() < 0.5 {
            return;
        }
        let mut prev = self.scene.elements[index].clone();
        prev.x = before.0;
        prev.y = before.1;
        self.undo.push(EditOp::Replace {
            before: prev,
            after: self.scene.elements[index].clone(),
            index,
        });
        self.redo.clear();
    }

    /// Recolour / restyle the selection with the toolbar's current style, as one undoable step.
    pub fn restyle_selected(&mut self) -> bool {
        let Some(id) = self.selected.clone() else {
            return false;
        };
        let Some(index) = self.index_of(&id) else {
            return false;
        };
        let before = self.scene.elements[index].clone();
        self.scene.elements[index].stroke = self.stroke;
        self.scene.elements[index].fill = self.fill;
        self.scene.elements[index].stroke_width = self.stroke_width;
        let after = self.scene.elements[index].clone();
        if before == after {
            return false;
        }
        self.undo.push(EditOp::Replace {
            before,
            after,
            index,
        });
        self.redo.clear();
        true
    }

    /// Commit a dragged rectangle: a shape, or the crop. A nothing-drag is ignored.
    pub fn commit_shape(&mut self, kind: ShapeKind, start: (f32, f32), end: (f32, f32)) -> bool {
        let (x0, y0, x1, y1) = normalise(start, end);
        match kind {
            ShapeKind::Crop => self.set_crop(x0, y0, x1, y1),
            ShapeKind::Copy => false,
            ShapeKind::Rect | ShapeKind::Ellipse | ShapeKind::Arrow => {
                if x1 - x0 < 2.0 && y1 - y0 < 2.0 {
                    return false;
                }
                let id = self.id();
                let element = match kind {
                    ShapeKind::Rect => self.styled(
                        id,
                        ElementKind::Rectangle { rounded: false },
                        x0,
                        y0,
                        x1 - x0,
                        y1 - y0,
                    ),
                    ShapeKind::Ellipse => {
                        self.styled(id, ElementKind::Ellipse, x0, y0, x1 - x0, y1 - y0)
                    }
                    _ => self.styled(
                        id,
                        ElementKind::Arrow {
                            points: vec![(0.0, 0.0), (x1 - x0, y1 - y0)],
                            start_arrow: false,
                            end_arrow: true,
                        },
                        x0,
                        y0,
                        x1 - x0,
                        y1 - y0,
                    ),
                };
                self.selected = Some(element.id.clone());
                self.push(element);
                true
            }
        }
    }

    /// Commit collected freehand points as one stroke.
    pub fn commit_draw(&mut self, points: Vec<(f32, f32)>) -> bool {
        if points.len() < 2 {
            return false;
        }
        let (x0, y0) = points[0];
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (x0, y0, x0, y0);
        for &(x, y) in &points[1..] {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        if max_x - min_x < 2.0 && max_y - min_y < 2.0 {
            return false;
        }
        let relative: Vec<(f32, f32)> = points.iter().map(|&(x, y)| (x - x0, y - y0)).collect();
        let id = self.id();
        let element = self.styled(
            id,
            ElementKind::FreeDraw { points: relative },
            x0,
            y0,
            max_x - min_x,
            max_y - min_y,
        );
        self.selected = Some(element.id.clone());
        self.push(element);
        true
    }

    /// The typed text, at the click that raised its dialog.
    pub fn append_text(&mut self, text: String, at: (f32, f32)) -> bool {
        if text.trim().is_empty() {
            return false;
        }
        let lines = text.lines().count().max(1) as f32;
        let longest = text.lines().map(|line| line.len()).max().unwrap_or(0) as f32;
        let size = 20.0;
        let id = self.id();
        let element = self.styled(
            id,
            ElementKind::Text {
                text,
                font_size: size,
                family: FontFamily::Normal,
                align: TextAlign::Left,
            },
            at.0,
            at.1,
            longest * size * 0.6,
            lines * size * 1.25,
        );
        self.selected = Some(element.id.clone());
        self.push(element);
        true
    }

    /// Narrow what is shown. Measured against the whole picture, so a crop is re-croppable:
    /// dragging wider brings back what an earlier crop took. A nothing-rect is ignored.
    pub fn set_crop(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
        let from = self.scene.bounds;
        let to = Bounds {
            min_x: x0.max(self.full.min_x).max(0.0),
            min_y: y0.max(self.full.min_y).max(0.0),
            max_x: x1.min(self.full.max_x).min(self.width as f32),
            max_y: y1.min(self.full.max_y).min(self.height as f32),
        };
        if to.width() < 2.0 || to.height() < 2.0 {
            return false;
        }
        if to == from {
            return false;
        }
        self.scene.bounds = to;
        self.undo.push(EditOp::Crop { from, to });
        self.redo.clear();
        true
    }

    pub fn undo(&mut self) -> bool {
        let Some(op) = self.undo.pop() else {
            return false;
        };
        match op.clone() {
            EditOp::Add { element } => {
                self.scene.elements.retain(|el| el.id != element.id);
                if self.selected.as_deref() == Some(&element.id) {
                    self.selected = None;
                }
            }
            EditOp::Remove { element, index } => {
                let at = index.min(self.scene.elements.len());
                self.scene.elements.insert(at, element);
            }
            EditOp::Replace { before, index, .. } => {
                if self
                    .scene
                    .elements
                    .get(index)
                    .is_some_and(|el| el.id == before.id)
                {
                    self.scene.elements[index] = before;
                }
            }
            EditOp::Crop { from, .. } => {
                self.scene.bounds = from;
            }
        }
        self.redo.push(op);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(op) = self.redo.pop() else {
            return false;
        };
        match op.clone() {
            EditOp::Add { element } => {
                self.scene.elements.push(element);
                self.scene
                    .elements
                    .sort_by_key(|element| element.kind.paint_rank());
            }
            EditOp::Remove { element, .. } => {
                self.scene.elements.retain(|el| el.id != element.id);
                if self.selected.as_deref() == Some(&element.id) {
                    self.selected = None;
                }
            }
            EditOp::Replace { after, index, .. } => {
                if self
                    .scene
                    .elements
                    .get(index)
                    .is_some_and(|el| el.id == after.id)
                {
                    self.scene.elements[index] = after;
                } else if let Some(at) = self.index_of(&after.id) {
                    self.scene.elements[at] = after;
                }
            }
            EditOp::Crop { to, .. } => {
                self.scene.bounds = to;
            }
        }
        self.undo.push(op);
        true
    }

    /// The scene plus the drag in progress, for the screen painter.
    pub fn display_scene(&self) -> Scene {
        let mut scene = self.scene.clone();
        if let Some(preview) = self.preview() {
            scene.elements.push(preview);
        }
        scene
    }

    fn preview(&self) -> Option<Element> {
        match self.pending.clone()? {
            Pending::Shape { kind, start, cur } => {
                let (x0, y0, x1, y1) = normalise(start, cur);
                let dashed = || StrokeStyle::Dashed;
                let frame = |id: &str| Element {
                    id: id.to_string(),
                    kind: ElementKind::Rectangle { rounded: false },
                    x: x0,
                    y: y0,
                    width: (x1 - x0).max(1.0),
                    height: (y1 - y0).max(1.0),
                    angle: 0.0,
                    stroke: Some(Rgba8::opaque(0xe0, 0x31, 0x31)),
                    fill: None,
                    stroke_width: 1.5,
                    stroke_style: dashed(),
                    opacity: 1.0,
                };
                match kind {
                    ShapeKind::Crop | ShapeKind::Copy => Some(frame("preview")),
                    ShapeKind::Rect => Some(self.styled(
                        "preview".to_string(),
                        ElementKind::Rectangle { rounded: false },
                        x0,
                        y0,
                        (x1 - x0).max(1.0),
                        (y1 - y0).max(1.0),
                    )),
                    ShapeKind::Ellipse => Some(self.styled(
                        "preview".to_string(),
                        ElementKind::Ellipse,
                        x0,
                        y0,
                        (x1 - x0).max(1.0),
                        (y1 - y0).max(1.0),
                    )),
                    ShapeKind::Arrow => Some(self.styled(
                        "preview".to_string(),
                        ElementKind::Arrow {
                            points: vec![(0.0, 0.0), (x1 - x0, y1 - y0)],
                            start_arrow: false,
                            end_arrow: true,
                        },
                        x0,
                        y0,
                        (x1 - x0).max(1.0),
                        (y1 - y0).max(1.0),
                    )),
                }
            }
            Pending::Draw { points } => {
                if points.len() < 2 {
                    return None;
                }
                let (x0, y0) = points[0];
                Some(self.styled(
                    "preview".to_string(),
                    ElementKind::FreeDraw {
                        points: points.iter().map(|&(x, y)| (x - x0, y - y0)).collect(),
                    },
                    x0,
                    y0,
                    1.0,
                    1.0,
                ))
            }
            Pending::Move { .. } => None,
        }
    }

    /// The save: base pixels cropped to the bounds, annotations over them at scene scale —
    /// scene units are base pixels — encoded PNG. PNG only.
    pub fn flatten(&self) -> Option<Vec<u8>> {
        let bounds = self.scene.bounds;
        self.flatten_rect(bounds.min_x, bounds.min_y, bounds.width(), bounds.height())
    }

    /// The same, over a sub-rectangle in scene coordinates: what copy-region writes.
    pub fn flatten_rect(&self, x: f32, y: f32, w: f32, h: f32) -> Option<Vec<u8>> {
        let base = image::load_from_memory(&self.base).ok()?.to_rgba8();
        let (bw, bh) = (base.width() as f32, base.height() as f32);
        let x0 = x.max(0.0).floor() as u32;
        let y0 = y.max(0.0).floor() as u32;
        let x1 = (x + w).min(bw).ceil() as u32;
        let y1 = (y + h).min(bh).ceil() as u32;
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let (w, h) = (x1 - x0, y1 - y0);
        let mut canvas = Canvas::cropped(&base, x0, y0, w, h);
        let mut ordered = self.scene.elements.clone();
        ordered.sort_by_key(|element| element.kind.paint_rank());
        for element in &ordered {
            if matches!(element.kind, ElementKind::Image { .. }) {
                continue;
            }
            draw_element(&mut canvas, element, x0 as f32, y0 as f32);
        }
        let raw = image::RgbaImage::from_raw(w, h, canvas.pixels)?;
        let mut out = Vec::new();
        raw.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .ok()?;
        Some(out)
    }
}

fn normalise(start: (f32, f32), end: (f32, f32)) -> (f32, f32, f32, f32) {
    (
        start.0.min(end.0),
        start.1.min(end.1),
        start.0.max(end.0),
        start.1.max(end.1),
    )
}

/// Whether a scene point is on an element. Connectors and strokes take a few pixels of grace;
/// rotation is not honoured — authored annotations are unrotated.
fn contains(element: &Element, x: f32, y: f32) -> bool {
    let (x0, y0, x1, y1) = (
        element.x.min(element.x + element.width),
        element.y.min(element.y + element.height),
        element.x.max(element.x + element.width),
        element.y.max(element.y + element.height),
    );
    match &element.kind {
        ElementKind::Line { points, .. }
        | ElementKind::Arrow { points, .. }
        | ElementKind::FreeDraw { points } => {
            let grace = (element.stroke_width.max(6.0)) / 2.0;
            points
                .iter()
                .map(|&(dx, dy)| (element.x + dx, element.y + dy))
                .collect::<Vec<_>>()
                .windows(2)
                .any(|pair| dist_to_segment(x, y, pair[0], pair[1]) <= grace)
        }
        _ => x >= x0 && x <= x1 && y >= y0 && y <= y1,
    }
}

fn dist_to_segment(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    if len2 < 0.001 {
        return ((px - a.0).powi(2) + (py - a.1).powi(2)).sqrt();
    }
    let t = (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0);
    ((px - (a.0 + t * dx)).powi(2) + (py - (a.1 + t * dy)).powi(2)).sqrt()
}

// ------------------------------------------------------------------------------------------- //
// The flatten rasterizer: the screen painter's subset, into bytes rather than onto glass.
// ------------------------------------------------------------------------------------------- //

/// RGBA pixels with src-over blending. Coordinates are canvas pixels; callers translate.
struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Canvas {
    fn cropped(base: &image::RgbaImage, x0: u32, y0: u32, w: u32, h: u32) -> Self {
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let src = base.get_pixel(x0 + x, y0 + y).0;
                let at = ((y * w + x) * 4) as usize;
                pixels[at..at + 4].copy_from_slice(&src);
            }
        }
        Self {
            width: w,
            height: h,
            pixels,
        }
    }

    fn dot(&mut self, x: i64, y: i64, colour: Rgba8, opacity: f32) {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return;
        }
        let at = ((y as u32 * self.width + x as u32) * 4) as usize;
        let src_a = (colour.a as f32 / 255.0) * opacity;
        let dst_a = self.pixels[at + 3] as f32 / 255.0;
        let out_a = src_a + dst_a * (1.0 - src_a);
        if out_a <= 0.0 {
            return;
        }
        for channel in 0..3 {
            let src = [colour.r, colour.g, colour.b][channel] as f32 / 255.0;
            let dst = self.pixels[at + channel] as f32 / 255.0;
            self.pixels[at + channel] =
                (((src * src_a + dst * dst_a * (1.0 - src_a)) / out_a) * 255.0) as u8;
        }
        self.pixels[at + 3] = (out_a * 255.0) as u8;
    }

    /// A square brush: strokes stay visible at any width without circle math per step.
    fn brush(&mut self, x: f32, y: f32, width: f32, colour: Rgba8, opacity: f32) {
        let r = (width / 2.0).max(0.5).ceil() as i64;
        for dy in -r..=r {
            for dx in -r..=r {
                self.dot(x as i64 + dx, y as i64 + dy, colour, opacity);
            }
        }
    }

    fn line(&mut self, a: (f32, f32), b: (f32, f32), width: f32, colour: Rgba8, opacity: f32) {
        let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil() as usize).max(1);
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            self.brush(
                a.0 + (b.0 - a.0) * t,
                a.1 + (b.1 - a.1) * t,
                width,
                colour,
                opacity,
            );
        }
    }

    fn fill_rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, colour: Rgba8, opacity: f32) {
        for y in y0.floor() as i64..=(y1.ceil() as i64) {
            for x in x0.floor() as i64..=(x1.ceil() as i64) {
                self.dot(x, y, colour, opacity);
            }
        }
    }

    /// Even-odd scanline fill, for diamonds and arrowheads.
    fn fill_poly(&mut self, points: &[(f32, f32)], colour: Rgba8, opacity: f32) {
        if points.len() < 3 {
            return;
        }
        let min_y = points
            .iter()
            .map(|p| p.1)
            .fold(f32::INFINITY, f32::min)
            .floor() as i64;
        let max_y = points
            .iter()
            .map(|p| p.1)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil() as i64;
        for y in min_y..=max_y {
            let mut crossings = Vec::new();
            for pair in points
                .iter()
                .zip(points.iter().cycle().skip(1))
                .take(points.len())
            {
                let (a, b) = (*pair.0, *pair.1);
                if (a.1 <= y as f32) != (b.1 <= y as f32) {
                    crossings.push(a.0 + (y as f32 - a.1) * (b.0 - a.0) / (b.1 - a.1));
                }
            }
            crossings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            for pair in crossings.chunks(2) {
                if pair.len() == 2 {
                    for x in pair[0].floor() as i64..=pair[1].ceil() as i64 {
                        self.dot(x, y, colour, opacity);
                    }
                }
            }
        }
    }
}

/// Paint one annotation at its scene position minus the crop origin.
fn draw_element(canvas: &mut Canvas, element: &Element, ox: f32, oy: f32) {
    let shift = |point: (f32, f32)| (point.0 - ox, point.1 - oy);
    let opacity = element.opacity;
    match &element.kind {
        ElementKind::Rectangle { .. } => {
            let (x0, y0) = shift((element.x, element.y));
            let (x1, y1) = shift((element.x + element.width, element.y + element.height));
            let (x0, x1) = (x0.min(x1), x0.max(x1));
            let (y0, y1) = (y0.min(y1), y0.max(y1));
            if let Some(fill) = element.fill {
                canvas.fill_rect(x0, y0, x1, y1, fill, opacity);
            }
            if let Some(stroke) = element.stroke {
                let w = element.stroke_width.max(1.0);
                canvas.line((x0, y0), (x1, y0), w, stroke, opacity);
                canvas.line((x1, y0), (x1, y1), w, stroke, opacity);
                canvas.line((x1, y1), (x0, y1), w, stroke, opacity);
                canvas.line((x0, y1), (x0, y0), w, stroke, opacity);
            }
        }
        ElementKind::Ellipse => {
            let (cx, cy) = shift((
                element.x + element.width / 2.0,
                element.y + element.height / 2.0,
            ));
            let (rx, ry) = (element.width.abs() / 2.0, element.height.abs() / 2.0);
            if rx < 1.0 || ry < 1.0 {
                return;
            }
            if let Some(fill) = element.fill {
                for row in 0..=(2.0 * ry).ceil() as i64 {
                    let dy = row as f32 - ry;
                    let half = rx * (1.0 - (dy * dy) / (ry * ry)).max(0.0).sqrt();
                    for col in (-half).floor() as i64..=half.ceil() as i64 {
                        canvas.dot(cx as i64 + col, (cy - ry) as i64 + row, fill, opacity);
                    }
                }
            }
            if let Some(stroke) = element.stroke {
                let steps = ((rx + ry) * 4.0).ceil() as usize;
                for step in 0..=steps {
                    let theta = std::f32::consts::TAU * step as f32 / steps as f32;
                    canvas.brush(
                        cx + rx * theta.cos(),
                        cy + ry * theta.sin(),
                        element.stroke_width.max(1.0),
                        stroke,
                        opacity,
                    );
                }
            }
        }
        ElementKind::Diamond => {
            let corners = [
                (element.x + element.width / 2.0, element.y),
                (element.x + element.width, element.y + element.height / 2.0),
                (element.x + element.width / 2.0, element.y + element.height),
                (element.x, element.y + element.height / 2.0),
            ];
            let mapped: Vec<(f32, f32)> = corners.iter().map(|&p| shift(p)).collect();
            if let Some(fill) = element.fill {
                canvas.fill_poly(&mapped, fill, opacity);
            }
            if let Some(stroke) = element.stroke {
                polyline(
                    canvas,
                    &mapped,
                    true,
                    element.stroke_width.max(1.0),
                    stroke,
                    opacity,
                );
            }
        }
        ElementKind::Line {
            points,
            start_arrow,
            end_arrow,
        }
        | ElementKind::Arrow {
            points,
            start_arrow,
            end_arrow,
        } => {
            let Some(stroke) = element.stroke else {
                return;
            };
            let mapped: Vec<(f32, f32)> = points
                .iter()
                .map(|&(dx, dy)| shift((element.x + dx, element.y + dy)))
                .collect();
            polyline(
                canvas,
                &mapped,
                false,
                element.stroke_width.max(1.0),
                stroke,
                opacity,
            );
            let head = 9.0 + element.stroke_width * 1.5;
            if *end_arrow && mapped.len() >= 2 {
                arrowhead(
                    canvas,
                    mapped[mapped.len() - 1],
                    mapped[mapped.len() - 2],
                    head,
                    stroke,
                    opacity,
                );
            }
            if *start_arrow && mapped.len() >= 2 {
                arrowhead(canvas, mapped[0], mapped[1], head, stroke, opacity);
            }
        }
        ElementKind::FreeDraw { points } => {
            let Some(stroke) = element.stroke else {
                return;
            };
            let mapped: Vec<(f32, f32)> = points
                .iter()
                .map(|&(dx, dy)| shift((element.x + dx, element.y + dy)))
                .collect();
            polyline(
                canvas,
                &mapped,
                false,
                element.stroke_width.max(1.0),
                stroke,
                opacity,
            );
        }
        ElementKind::Text {
            text, font_size, ..
        } => {
            let Some(stroke) = element.stroke else {
                return;
            };
            let (x0, y0) = shift((element.x, element.y));
            draw_text(canvas, text, x0, y0, *font_size, stroke, opacity);
        }
        // The base image is the canvas already; frames group, they do not paint.
        ElementKind::Image { .. } | ElementKind::Frame { .. } => {}
    }
}

fn polyline(
    canvas: &mut Canvas,
    points: &[(f32, f32)],
    closed: bool,
    width: f32,
    colour: Rgba8,
    opacity: f32,
) {
    let n = points.len();
    if n < 2 {
        return;
    }
    for pair in points.windows(2) {
        canvas.line(pair[0], pair[1], width, colour, opacity);
    }
    if closed {
        canvas.line(points[n - 1], points[0], width, colour, opacity);
    }
}

/// A solid triangular head, the size the screen painter uses.
fn arrowhead(
    canvas: &mut Canvas,
    tip: (f32, f32),
    from: (f32, f32),
    size: f32,
    colour: Rgba8,
    opacity: f32,
) {
    let (dx, dy) = (tip.0 - from.0, tip.1 - from.1);
    let len = dx.hypot(dy);
    if len < 0.001 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    canvas.fill_poly(
        &[
            tip,
            (
                tip.0 - ux * size - uy * size * 0.45,
                tip.1 - uy * size + ux * size * 0.45,
            ),
            (
                tip.0 - ux * size + uy * size * 0.45,
                tip.1 - uy * size - ux * size * 0.45,
            ),
        ],
        colour,
        opacity,
    );
}

// A 5x7 bitmap for printable ASCII, one row per byte, MSB left. Text in a flattened PNG has no
// shaper behind it; this keeps a label a label rather than dropping it silently.
// ponytail: blocky at large sizes — a real font when text flattening matters.
const GLYPHS: [[u8; 7]; 95] = [
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // space
    [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04], // !
    [0x0A, 0x0A, 0x0A, 0x00, 0x00, 0x00, 0x00], // "
    [0x0A, 0x0A, 0x1F, 0x0A, 0x1F, 0x0A, 0x0A], // #
    [0x04, 0x0F, 0x14, 0x0E, 0x05, 0x1E, 0x04], // $
    [0x19, 0x1A, 0x02, 0x04, 0x08, 0x0B, 0x13], // %
    [0x0C, 0x12, 0x14, 0x08, 0x15, 0x12, 0x0D], // &
    [0x04, 0x04, 0x08, 0x00, 0x00, 0x00, 0x00], // '
    [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02], // (
    [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08], // )
    [0x00, 0x04, 0x15, 0x0E, 0x15, 0x04, 0x00], // *
    [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00], // +
    [0x00, 0x00, 0x00, 0x00, 0x04, 0x04, 0x08], // ,
    [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00], // -
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x04], // .
    [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10], // /
    [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E], // 0
    [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E], // 1
    [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F], // 2
    [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E], // 3
    [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02], // 4
    [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E], // 5
    [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E], // 6
    [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08], // 7
    [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E], // 8
    [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C], // 9
    [0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x00], // :
    [0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x08], // ;
    [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02], // <
    [0x00, 0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00], // =
    [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08], // >
    [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04], // ?
    [0x0E, 0x11, 0x01, 0x0D, 0x15, 0x15, 0x0E], // @
    [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11], // A
    [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E], // B
    [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E], // C
    [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E], // D
    [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F], // E
    [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10], // F
    [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F], // G
    [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11], // H
    [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E], // I
    [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C], // J
    [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11], // K
    [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F], // L
    [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11], // M
    [0x11, 0x19, 0x19, 0x15, 0x13, 0x13, 0x11], // N
    [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E], // O
    [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10], // P
    [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D], // Q
    [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11], // R
    [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E], // S
    [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04], // T
    [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E], // U
    [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04], // V
    [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11], // W
    [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11], // X
    [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04], // Y
    [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F], // Z
    [0x0E, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0E], // [
    [0x10, 0x08, 0x08, 0x04, 0x02, 0x02, 0x01], // backslash
    [0x0E, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0E], // ]
    [0x04, 0x0A, 0x11, 0x00, 0x00, 0x00, 0x00], // ^
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F], // _
    [0x08, 0x04, 0x02, 0x00, 0x00, 0x00, 0x00], // `
    [0x00, 0x00, 0x0E, 0x01, 0x0F, 0x11, 0x0F], // a
    [0x10, 0x10, 0x1E, 0x11, 0x11, 0x11, 0x1E], // b
    [0x00, 0x00, 0x0E, 0x10, 0x10, 0x11, 0x0E], // c
    [0x01, 0x01, 0x0F, 0x11, 0x11, 0x11, 0x0F], // d
    [0x00, 0x00, 0x0E, 0x11, 0x1F, 0x10, 0x0E], // e
    [0x06, 0x08, 0x08, 0x1E, 0x08, 0x08, 0x08], // f
    [0x00, 0x00, 0x0F, 0x11, 0x0F, 0x01, 0x0E], // g
    [0x10, 0x10, 0x1E, 0x11, 0x11, 0x11, 0x11], // h
    [0x04, 0x00, 0x0C, 0x04, 0x04, 0x04, 0x0E], // i
    [0x02, 0x00, 0x06, 0x02, 0x02, 0x12, 0x0C], // j
    [0x10, 0x10, 0x12, 0x14, 0x18, 0x14, 0x12], // k
    [0x0C, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E], // l
    [0x00, 0x00, 0x1A, 0x15, 0x15, 0x11, 0x11], // m
    [0x00, 0x00, 0x1E, 0x11, 0x11, 0x11, 0x11], // n
    [0x00, 0x00, 0x0E, 0x11, 0x11, 0x11, 0x0E], // o
    [0x00, 0x00, 0x1E, 0x11, 0x1E, 0x10, 0x10], // p
    [0x00, 0x00, 0x0F, 0x11, 0x0F, 0x01, 0x01], // q
    [0x00, 0x00, 0x16, 0x19, 0x10, 0x10, 0x10], // r
    [0x00, 0x00, 0x0F, 0x10, 0x0E, 0x01, 0x1E], // s
    [0x08, 0x08, 0x1E, 0x08, 0x08, 0x09, 0x06], // t
    [0x00, 0x00, 0x11, 0x11, 0x11, 0x13, 0x0D], // u
    [0x00, 0x00, 0x11, 0x11, 0x11, 0x0A, 0x04], // v
    [0x00, 0x00, 0x11, 0x15, 0x15, 0x1B, 0x11], // w (approx)
    [0x00, 0x00, 0x11, 0x0A, 0x04, 0x0A, 0x11], // x
    [0x00, 0x00, 0x11, 0x11, 0x0F, 0x01, 0x0E], // y
    [0x00, 0x00, 0x1F, 0x02, 0x04, 0x08, 0x1F], // z
    [0x02, 0x04, 0x04, 0x08, 0x04, 0x04, 0x02], // {
    [0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04], // |
    [0x08, 0x04, 0x04, 0x02, 0x04, 0x04, 0x08], // }
    [0x00, 0x0E, 0x11, 0x00, 0x00, 0x00, 0x00], // ~
];

fn draw_text(
    canvas: &mut Canvas,
    text: &str,
    x0: f32,
    y0: f32,
    font_size: f32,
    colour: Rgba8,
    opacity: f32,
) {
    let scale = (font_size / 7.0).round().max(1.0) as i64;
    let (mut cx, mut cy) = (x0 as i64, y0 as i64);
    for ch in text.chars() {
        if ch == '\n' {
            cx = x0 as i64;
            cy += 8 * scale;
            continue;
        }
        if ch == ' ' {
            cx += 6 * scale;
            continue;
        }
        if let Some(glyph) = glyph(ch) {
            for (row, bits) in glyph.iter().enumerate() {
                for col in 0..5 {
                    if bits & (1 << (4 - col)) != 0 {
                        for dy in 0..scale {
                            for dx in 0..scale {
                                canvas.dot(
                                    cx + col * scale + dx,
                                    cy + row as i64 * scale + dy,
                                    colour,
                                    opacity,
                                );
                            }
                        }
                    }
                }
            }
        }
        cx += 6 * scale;
    }
}

fn glyph(ch: char) -> Option<&'static [u8; 7]> {
    let code = ch as u32;
    if (32..127).contains(&code) {
        Some(&GLYPHS[(code - 32) as usize])
    } else {
        None
    }
}
