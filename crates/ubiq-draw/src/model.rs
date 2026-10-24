//! The format-agnostic drawing: what every converter produces and the one painter reads.
//!
//! Three rules a painter can rely on without asking a format:
//!
//! - **The tree is the paint order.** [`Drawing::root`] is walked depth-first, children front to
//!   back; a [`Group`] is a contiguous run of the stack and never reorders anything.
//! - **Coordinates are absolute**, in the file's own units. A connector's points and a polyline's
//!   points are absolute too; [`Shape::frame`] is the box rotation turns about and a text box is
//!   laid into.
//! - **`None` on a colour is transparent**, not "use a default". Each converter applies its
//!   format's defaults while converting.

use std::collections::HashMap;

use crate::colour::Rgba8;

/// The largest magnitude a coordinate or size is given: a number past it (or not finite) in a
/// file is malformed, and both converters clamp to it so nothing downstream sees an infinity.
pub const MAX_COORD: f32 = 1.0e6;

/// The largest font size a label is given; a bigger one is a malformed file.
pub const MAX_FONT_SIZE: f32 = 400.0;

/// A point in drawing units.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// An axis-aligned box. Width and height may be negative in a source file; converters normalise.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn centre(&self) -> Point {
        Point::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// How a stroke is dashed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum StrokeStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

/// The three families a painter can tell apart. A format's hand-drawn face falls back to the
/// interface's own.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FontFamily {
    Hand,
    #[default]
    Normal,
    Code,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum VAlign {
    #[default]
    Top,
    Middle,
    Bottom,
}

/// Stroke, fill, opacity and dash — everything about a shape's ink.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Style {
    /// `None` is no outline.
    pub stroke: Option<Rgba8>,
    /// `None` is hollow.
    pub fill: Option<Rgba8>,
    pub stroke_width: f32,
    pub stroke_style: StrokeStyle,
    /// `0.0` to `1.0`, applied to stroke, fill and text alike.
    pub opacity: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            stroke: Some(Rgba8::DEFAULT_STROKE),
            fill: None,
            stroke_width: 1.0,
            stroke_style: StrokeStyle::Solid,
            opacity: 1.0,
        }
    }
}

/// Text laid into a box. A shape's label and a free text element are both one of these.
#[derive(Clone, PartialEq, Debug)]
pub struct TextBox {
    /// Plain text; `\n` separates lines. Markup is stripped by the converter.
    pub text: String,
    pub rect: Rect,
    pub font_size: f32,
    pub family: FontFamily,
    pub align: TextAlign,
    pub valign: VAlign,
    /// `None` is invisible text.
    pub colour: Option<Rgba8>,
}

/// What an arrow's end looks like.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ArrowHead {
    #[default]
    None,
    /// A solid triangle.
    Filled,
    /// A stroked chevron.
    Open,
}

/// A line between two things, with heads and, when the file said so, the shapes it joins.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Connector {
    /// Absolute, at least two to draw.
    pub points: Vec<Point>,
    pub start: ArrowHead,
    pub end: ArrowHead,
    /// The id of the shape the start is bound to.
    pub source: Option<String>,
    /// The id of the shape the end is bound to.
    pub target: Option<String>,
}

/// The outline a shape draws. Text is not a geometry of its own beyond [`Geometry::Text`]: any
/// shape may carry a [`TextBox`].
#[derive(Clone, PartialEq, Debug)]
pub enum Geometry {
    Rect,
    RoundedRect {
        radius: f32,
    },
    Ellipse,
    Diamond,
    /// Absolute points; `closed` joins the last to the first and lets a fill apply.
    Polyline {
        points: Vec<Point>,
        closed: bool,
    },
    Connector(Connector),
    /// A freehand stroke through absolute points.
    Freehand {
        points: Vec<Point>,
    },
    /// No outline: the shape is its text box.
    Text,
    /// A picture whose bytes are in [`Drawing::images`] under this id, or `None` (or a missing
    /// id) for one the file did not carry — painted as a placeholder.
    Image {
        image: Option<String>,
    },
    /// Something the converter could not draw. Painted as a dashed box with the shape's label, so
    /// an unsupported element is visible rather than absent.
    Placeholder,
}

/// One drawable thing.
#[derive(Clone, PartialEq, Debug)]
pub struct Shape {
    /// The file's own id.
    pub id: String,
    pub geometry: Geometry,
    /// The box the geometry is laid into and rotated about. For a connector, the extent of its
    /// points.
    pub frame: Rect,
    /// Clockwise, radians, about [`Rect::centre`] of [`Shape::frame`]. Text is never rotated.
    pub rotation: f32,
    pub style: Style,
    pub text: Option<TextBox>,
}

/// A contiguous run of the stack: a group, a frame with its children, a draw.io layer or
/// container. Its children are in paint order.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Group {
    pub id: String,
    pub children: Vec<Node>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Node {
    Shape(Shape),
    Group(Group),
}

/// An image the file carried inline.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EmbeddedImage {
    /// `image/png`, `image/svg+xml`, … — whatever the URI declared.
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// The extent of what is drawn, unpadded. The renderer adds its own margin.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Bounds {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

impl Bounds {
    pub const EMPTY: Self = Self {
        min_x: 0.0,
        min_y: 0.0,
        max_x: 0.0,
        max_y: 0.0,
    };

    pub fn width(&self) -> f32 {
        self.max_x - self.min_x
    }

    pub fn height(&self) -> f32 {
        self.max_y - self.min_y
    }

    pub fn is_empty(&self) -> bool {
        self.width() <= 0.0 && self.height() <= 0.0
    }

    /// The union of a set of boxes, or [`Bounds::EMPTY`] for none.
    pub fn of(rects: impl IntoIterator<Item = Rect>) -> Self {
        let mut bounds: Option<Bounds> = None;
        for r in rects {
            let next = Bounds {
                min_x: r.x.min(r.x + r.w),
                min_y: r.y.min(r.y + r.h),
                max_x: r.x.max(r.x + r.w),
                max_y: r.y.max(r.y + r.h),
            };
            bounds = Some(match bounds {
                None => next,
                Some(so_far) => Bounds {
                    min_x: so_far.min_x.min(next.min_x),
                    min_y: so_far.min_y.min(next.min_y),
                    max_x: so_far.max_x.max(next.max_x),
                    max_y: so_far.max_y.max(next.max_y),
                },
            });
        }
        bounds.unwrap_or(Bounds::EMPTY)
    }
}

/// A whole drawing, ready to paint.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Drawing {
    /// Walk depth-first: this **is** the paint order.
    pub root: Group,
    /// The canvas colour the file named. `None` is "the file named none", and the painter then
    /// uses [`Rgba8::DEFAULT_BACKGROUND`] rather than the window's ground.
    pub background: Option<Rgba8>,
    pub images: HashMap<String, EmbeddedImage>,
    /// The viewport a fit should show.
    pub bounds: Bounds,
}

impl Drawing {
    /// Every shape in paint order.
    pub fn shapes(&self) -> Vec<&Shape> {
        fn walk<'a>(group: &'a Group, out: &mut Vec<&'a Shape>) {
            for node in &group.children {
                match node {
                    Node::Shape(shape) => out.push(shape),
                    Node::Group(inner) => walk(inner, out),
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }

    pub fn is_empty(&self) -> bool {
        self.shapes().is_empty()
    }
}
