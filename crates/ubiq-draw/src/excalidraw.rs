//! An Excalidraw file, parsed into its own element list, and mapped onto the drawing model.
//!
//! A `.excalidraw` file is JSON with a closed vocabulary — not a language — so the viewer needs no
//! renderer in another process: it needs the file turned into shapes. [`Scene`] is that list, in
//! Excalidraw's own terms; it is also what the image editor edits, which is why it stays a type of
//! its own. [`Scene::to_drawing`] is the map onto [`crate::model`], and the painter only ever sees
//! the result.
//!
//! Three rules the file format does not make obvious:
//!
//! - **Paint order is Excalidraw's own: the array is the stack, and a group stays together.**
//!   A group — and a frame with its children — is drawn as one contiguous run at the position of
//!   its first (lowest) member, as `defragmentGroups` does, nested groups following `groupIds` (innermost first), a frame under its
//!   own children, and a bound label (`containerId`) right after its container. The sort happens
//!   here, once ([`stacking_order`]), so [`Scene::elements`] is already in paint order.
//! - **An element this module does not know is skipped, and the rest of the scene parses.** One
//!   unknown type is a missing shape, never a blank panel.
//! - **`None` on a colour means transparent**, not "use a default". The defaults are applied while
//!   parsing, so an absent `strokeColor` arrives as Excalidraw's `#1e1e1e` and an explicit
//!   `"transparent"` arrives as `None`.

use std::borrow::Cow;
use std::collections::HashMap;

use serde_json::Value;

use crate::codec;
pub use crate::colour::Rgba8;
use crate::model::{
    self, ArrowHead, Connector, Drawing, Geometry, Group, Node, Point, Rect, Shape,
};
pub use crate::model::{Bounds, EmbeddedImage as EmbeddedFile, FontFamily, StrokeStyle, TextAlign};
use crate::model::{MAX_COORD, MAX_FONT_SIZE};

/// Excalidraw's own corner radius for a rounded rectangle, matching the reference renderer's `rx`.
pub const CORNER: f32 = 12.0;

/// The size a frame's name is drawn at, above its top-left corner.
const FRAME_NAME_SIZE: f32 = 14.0;

/// How many `groupIds` an element keeps; a deeper nesting is a malformed file.
const MAX_GROUP_DEPTH: usize = 32;

/// What an element is, and the fields only that kind has.
///
/// A kind this module does not name is not a variant: it is an element the parser skipped.
#[derive(Clone, PartialEq, Debug)]
pub enum ElementKind {
    Rectangle {
        rounded: bool,
    },
    Ellipse,
    Diamond,
    /// A named region drawn behind its contents. The name is drawn above the top-left corner.
    Frame {
        name: Option<String>,
    },
    Line {
        points: Vec<(f32, f32)>,
        start_arrow: bool,
        end_arrow: bool,
    },
    Arrow {
        points: Vec<(f32, f32)>,
        start_arrow: bool,
        end_arrow: bool,
    },
    Text {
        text: String,
        font_size: f32,
        family: FontFamily,
        align: TextAlign,
    },
    FreeDraw {
        points: Vec<(f32, f32)>,
    },
    /// An image whose bytes are in [`Scene::files`] under this id — or are missing, if the file
    /// carried no `files` map.
    Image {
        file_id: String,
    },
}

impl ElementKind {
    /// The image editor's stack: frames under everything, then shapes, then connectors, then
    /// text, equal ranks in the order they were drawn. An annotation scene has no groups, so this
    /// is its whole ordering rule; a parsed file is ordered by [`stacking_order`] instead.
    pub fn paint_rank(&self) -> u8 {
        match self {
            ElementKind::Frame { .. } => 0,
            ElementKind::Rectangle { .. } | ElementKind::Ellipse | ElementKind::Diamond => 1,
            ElementKind::Line { .. } | ElementKind::Arrow { .. } => 2,
            ElementKind::Text { .. } => 3,
            ElementKind::FreeDraw { .. } | ElementKind::Image { .. } => 1,
        }
    }

    /// The points of a connector or a stroke, relative to the element's origin.
    pub fn points(&self) -> &[(f32, f32)] {
        match self {
            ElementKind::Line { points, .. }
            | ElementKind::Arrow { points, .. }
            | ElementKind::FreeDraw { points } => points,
            _ => &[],
        }
    }
}

/// One drawable thing: a box the renderer places, plus the kind that says what goes in it.
#[derive(Clone, PartialEq, Debug)]
pub struct Element {
    /// The file's own id, kept for hit-testing and for saying which element went wrong.
    pub id: String,
    pub kind: ElementKind,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Rotation about the element's centre, in radians, as the file stores it.
    pub angle: f32,
    /// `None` is transparent, not a default.
    pub stroke: Option<Rgba8>,
    pub fill: Option<Rgba8>,
    pub stroke_width: f32,
    pub stroke_style: StrokeStyle,
    /// `0.0` to `1.0`. The file writes `0` to `100`.
    pub opacity: f32,
    /// What the element is bound into: its groups, its frame, its container, its ends.
    pub binding: Binding,
}

/// What ties an element to others — and therefore decides where it stacks, and what an arrow
/// joins.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Binding {
    /// `groupIds`, innermost first, as the file stores them.
    pub group_ids: Vec<String>,
    /// `frameId`: the frame this element sits in.
    pub frame_id: Option<String>,
    /// `containerId`: the shape a bound label belongs to.
    pub container_id: Option<String>,
    /// `startBinding.elementId`: the shape an arrow starts on.
    pub start: Option<String>,
    /// `endBinding.elementId`: the shape an arrow ends on.
    pub end: Option<String>,
}

/// A parsed scene: elements in paint order, the canvas colour, the embedded files, the extent.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Scene {
    /// **Already in paint order.** Walk it front to back.
    pub elements: Vec<Element>,
    /// `appState.viewBackgroundColor`. `None` is "the file named none", and the painter then uses
    /// [`Rgba8::DEFAULT_BACKGROUND`] — Excalidraw's white canvas — rather than the window's ground.
    pub background: Option<Rgba8>,
    pub files: HashMap<String, EmbeddedFile>,
    /// The union of each element's `x, y, x + width, y + height` — so, as in the reference
    /// renderer, a connector whose points reach outside its own box is not accounted for.
    pub bounds: Bounds,
}

/// Why a file did not become a scene. One bad element is never one of these.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SceneError {
    /// The bytes were not text.
    NotUtf8,
    /// A Markdown scene whose drawing is in a `compressed-json` fence. Obsidian's plugin writes
    /// these; decompressing one is not implemented, and the viewer says so rather than guessing.
    Compressed,
    /// Neither JSON nor a Markdown file with a drawing in it.
    NotAScene,
    /// The JSON did not parse.
    Json(String),
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SceneError::NotUtf8 => write!(f, "this file is not text"),
            SceneError::Compressed => write!(f, "this scene is stored compressed"),
            SceneError::NotAScene => write!(f, "this file holds no Excalidraw scene"),
            SceneError::Json(why) => write!(f, "this scene did not parse: {why}"),
        }
    }
}

impl std::error::Error for SceneError {}

impl Scene {
    /// Parse a `.excalidraw` file, or the `.excalidraw.md` Markdown variant.
    ///
    /// Plain JSON is taken as the scene. Otherwise the file is read as Markdown and the drawing is
    /// the fenced block under `## Drawing` — `json` is parsed, `compressed-json` is
    /// [`SceneError::Compressed`].
    pub fn parse(bytes: &[u8]) -> Result<Self, SceneError> {
        let text = std::str::from_utf8(bytes).map_err(|_| SceneError::NotUtf8)?;
        let head = text.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
        let json: Cow<'_, str> = if head.starts_with('{') {
            Cow::Borrowed(head)
        } else {
            Cow::Owned(drawing_fence(text)?)
        };
        let value: Value =
            serde_json::from_str(&json).map_err(|why| SceneError::Json(why.to_string()))?;
        Ok(Self::from_json(&value))
    }

    /// Build a scene from an already-parsed document. Public because a `excalidraw` fence inside a
    /// Markdown preview arrives as JSON someone else already read.
    pub fn from_json(value: &Value) -> Self {
        let elements: Vec<Element> = value
            .get("elements")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter(|raw| {
                !raw.get("isDeleted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .filter_map(element)
            .collect();

        let bounds = Bounds::of(elements.iter().map(element_box));
        let order = stacking_order(&elements);
        let mut slots: Vec<Option<Element>> = elements.into_iter().map(Some).collect();
        let elements: Vec<Element> = order.into_iter().filter_map(|i| slots[i].take()).collect();

        let background = value
            .get("appState")
            .and_then(|state| state.get("viewBackgroundColor"))
            .and_then(Value::as_str)
            .and_then(Rgba8::parse);

        Scene {
            elements,
            background,
            files: embedded_files(value),
            bounds,
        }
    }

    /// The scene on the drawing model: the group tree [`stacking_order`] implies, one
    /// [`Shape`] per element.
    pub fn to_drawing(&self) -> Drawing {
        fn node(run: Run, elements: &[Element]) -> Node {
            match run {
                Run::One(i) => Node::Shape(shape(&elements[i])),
                Run::Group(id, runs) => Node::Group(Group {
                    id,
                    children: runs.into_iter().map(|r| node(r, elements)).collect(),
                }),
            }
        }
        Drawing {
            root: Group {
                id: String::new(),
                children: stack_tree(&self.elements)
                    .into_iter()
                    .map(|run| node(run, &self.elements))
                    .collect(),
            },
            background: self.background,
            images: self.files.clone(),
            bounds: self.bounds,
        }
    }
}

fn element_box(element: &Element) -> Rect {
    Rect::new(element.x, element.y, element.width, element.height)
}

/// One element on the model. Points become absolute here; the box stays as the file wrote it,
/// because it is what `angle` turns about.
fn shape(element: &Element) -> Shape {
    let frame = element_box(element);
    let mut style = model::Style {
        stroke: element.stroke,
        fill: element.fill,
        stroke_width: element.stroke_width,
        stroke_style: element.stroke_style,
        opacity: element.opacity,
    };
    let absolute = |points: &[(f32, f32)]| -> Vec<Point> {
        points
            .iter()
            .map(|&(dx, dy)| Point::new(element.x + dx, element.y + dy))
            .collect()
    };
    let head = |on: bool| {
        if on {
            ArrowHead::Filled
        } else {
            ArrowHead::None
        }
    };
    let mut text = None;

    let geometry = match &element.kind {
        ElementKind::Rectangle { rounded: true } => Geometry::RoundedRect { radius: CORNER },
        ElementKind::Rectangle { rounded: false } => Geometry::Rect,
        ElementKind::Ellipse => Geometry::Ellipse,
        ElementKind::Diamond => Geometry::Diamond,
        ElementKind::Frame { name } => {
            // A frame is a grouping: a thin dashed box, its name above the top-left corner.
            let colour = element.stroke.unwrap_or(Rgba8::DEFAULT_STROKE);
            style.stroke = Some(colour);
            style.fill = None;
            style.stroke_width = 1.0;
            style.stroke_style = StrokeStyle::Dashed;
            text = name.as_ref().map(|name| model::TextBox {
                text: name.clone(),
                rect: Rect::new(
                    element.x,
                    element.y - FRAME_NAME_SIZE * 1.4,
                    element.width.abs(),
                    FRAME_NAME_SIZE * 1.25,
                ),
                font_size: FRAME_NAME_SIZE,
                family: FontFamily::Normal,
                align: TextAlign::Left,
                valign: model::VAlign::Top,
                colour: Some(colour),
            });
            Geometry::Rect
        }
        ElementKind::Line {
            points,
            start_arrow,
            end_arrow,
        } if !start_arrow && !end_arrow => {
            let points = absolute(points);
            // A line whose ends meet is a polygon in Excalidraw, and its background fills it.
            let closed = points.len() > 2
                && points
                    .first()
                    .zip(points.last())
                    .is_some_and(|(a, b)| (a.x - b.x).abs() < 0.5 && (a.y - b.y).abs() < 0.5);
            Geometry::Polyline { points, closed }
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
        } => Geometry::Connector(Connector {
            points: absolute(points),
            start: head(*start_arrow),
            end: head(*end_arrow),
            source: element.binding.start.clone(),
            target: element.binding.end.clone(),
        }),
        ElementKind::FreeDraw { points } => Geometry::Freehand {
            points: absolute(points),
        },
        ElementKind::Text {
            text: body,
            font_size,
            family,
            align,
        } => {
            text = Some(model::TextBox {
                text: body.clone(),
                rect: frame,
                font_size: *font_size,
                family: *family,
                align: *align,
                valign: model::VAlign::Top,
                colour: element.stroke,
            });
            Geometry::Text
        }
        ElementKind::Image { file_id } => Geometry::Image {
            image: Some(file_id.clone()),
        },
    };

    Shape {
        id: element.id.clone(),
        geometry,
        frame,
        rotation: element.angle,
        style,
        text,
    }
}

/// One run of the stack: an element, or a group of runs.
enum Run {
    One(usize),
    Group(String, Vec<Run>),
}

/// The order Excalidraw stacks `elements` in, as indices into the slice (which is file order).
///
/// - **The array is the z-order.** Nothing else — not the type, not `index` — moves an element.
/// - **A group is one contiguous run, placed where its first (lowest) member is** — Excalidraw's
///   `defragmentGroups` (`packages/element/src/sortElements.ts`). Its members keep their relative
///   order inside it, and a nested group is a run inside that run: `groupIds` is innermost
///   first, so the chain is read back to front.
/// - **A frame and the elements whose `frameId` names it are a run too, the frame first** — it is
///   the ground its children sit on.
/// - **A bound label draws right after its container**, whatever position the file gave it.
///
/// Every index appears exactly once, so a scene loses nothing to an odd binding: a `frameId` or a
/// `containerId` naming an element that is not there is ignored.
pub fn stacking_order(elements: &[Element]) -> Vec<usize> {
    fn flatten(runs: Vec<Run>, out: &mut Vec<usize>) {
        for run in runs {
            match run {
                Run::One(i) => out.push(i),
                Run::Group(_, inner) => flatten(inner, out),
            }
        }
    }
    let mut order = Vec::with_capacity(elements.len());
    flatten(stack_tree(elements), &mut order);
    order
}

/// The stack as a tree of runs — [`stacking_order`] flattened is this walked depth-first.
fn stack_tree(elements: &[Element]) -> Vec<Run> {
    const FRAME: &str = "\u{0}frame:";
    let by_id: HashMap<&str, usize> = elements
        .iter()
        .enumerate()
        .map(|(i, element)| (element.id.as_str(), i))
        .collect();
    let is_frame = |i: usize| matches!(elements[i].kind, ElementKind::Frame { .. });

    // Bound labels leave the stack and ride behind their container.
    let mut labels: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut free: Vec<usize> = Vec::with_capacity(elements.len());
    for (i, element) in elements.iter().enumerate() {
        let container = element
            .binding
            .container_id
            .as_deref()
            .and_then(|id| by_id.get(id).copied())
            .filter(|&c| c != i && elements[c].binding.container_id.is_none());
        match container {
            Some(c) => labels.entry(c).or_default().push(i),
            None => free.push(i),
        }
    }

    // Each element's chain of runs, outermost first: its frame, then its groups.
    let chains: Vec<Vec<String>> = elements
        .iter()
        .enumerate()
        .map(|(i, element)| {
            if is_frame(i) {
                return vec![format!("{FRAME}{}", element.id)];
            }
            let mut chain = Vec::with_capacity(element.binding.group_ids.len() + 1);
            if let Some(frame) = element.binding.frame_id.as_deref()
                && by_id.get(frame).is_some_and(|&f| is_frame(f))
            {
                chain.push(format!("{FRAME}{frame}"));
            }
            chain.extend(element.binding.group_ids.iter().rev().cloned());
            chain
        })
        .collect();

    struct Ctx<'a> {
        elements: &'a [Element],
        chains: &'a [Vec<String>],
        labels: HashMap<usize, Vec<usize>>,
    }

    fn stack(members: Vec<usize>, depth: usize, ctx: &mut Ctx<'_>) -> Vec<Run> {
        // Runs in first-seen order: a group takes the slot of its first (lowest) member, as
        // Excalidraw's `defragmentGroups` does.
        let mut runs: Vec<(Option<String>, Vec<usize>)> = Vec::new();
        let chains = ctx.chains;
        let mut slot: HashMap<&str, usize> = HashMap::new();
        for m in members {
            match chains[m].get(depth) {
                Some(key) => match slot.get(key.as_str()) {
                    Some(&at) => runs[at].1.push(m),
                    None => {
                        slot.insert(key.as_str(), runs.len());
                        runs.push((Some(key.clone()), vec![m]));
                    }
                },
                None => runs.push((None, vec![m])),
            }
        }

        let mut out = Vec::with_capacity(runs.len());
        for (key, mut run) in runs {
            match key {
                None => {
                    for m in run {
                        out.push(Run::One(m));
                        for label in ctx.labels.remove(&m).unwrap_or_default() {
                            out.push(Run::One(label));
                        }
                    }
                }
                Some(key) => {
                    let mut inner = Vec::new();
                    let id = match key.strip_prefix(FRAME) {
                        Some(frame) => {
                            if let Some(at) = run.iter().position(|&m| {
                                ctx.elements[m].id == frame
                                    && matches!(ctx.elements[m].kind, ElementKind::Frame { .. })
                            }) {
                                let m = run.remove(at);
                                inner.push(Run::One(m));
                                for label in ctx.labels.remove(&m).unwrap_or_default() {
                                    inner.push(Run::One(label));
                                }
                            }
                            frame.to_string()
                        }
                        None => key.clone(),
                    };
                    inner.extend(stack(run, depth + 1, ctx));
                    out.push(Run::Group(id, inner));
                }
            }
        }
        out
    }

    let mut ctx = Ctx {
        elements,
        chains: &chains,
        labels,
    };
    let mut runs = stack(free, 0, &mut ctx);
    // Unreachable for a well-formed binding; kept so no element is ever dropped.
    let mut left: Vec<usize> = ctx.labels.into_values().flatten().collect();
    left.sort_unstable();
    runs.extend(left.into_iter().map(Run::One));
    runs
}

/// One element, or `None` if its type is one this module does not draw.
fn element(raw: &Value) -> Option<Element> {
    let kind = kind(raw)?;
    Some(Element {
        id: raw
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        kind,
        x: number(raw, "x", 0.0),
        y: number(raw, "y", 0.0),
        width: number(raw, "width", 0.0),
        height: number(raw, "height", 0.0),
        angle: number(raw, "angle", 0.0),
        stroke: colour(raw, "strokeColor", Some(Rgba8::DEFAULT_STROKE)),
        fill: colour(raw, "backgroundColor", None),
        stroke_width: number(raw, "strokeWidth", 2.0),
        stroke_style: match raw.get("strokeStyle").and_then(Value::as_str) {
            Some("dashed") => StrokeStyle::Dashed,
            Some("dotted") => StrokeStyle::Dotted,
            _ => StrokeStyle::Solid,
        },
        opacity: (number(raw, "opacity", 100.0) / 100.0).clamp(0.0, 1.0),
        binding: Binding {
            group_ids: raw
                .get("groupIds")
                .and_then(Value::as_array)
                .map(|ids| {
                    let ids: Vec<String> = ids
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect();
                    // Innermost first: keep the outermost `MAX_GROUP_DEPTH`, so the stack's
                    // recursion stays bounded whatever the file says.
                    let skip = ids.len().saturating_sub(MAX_GROUP_DEPTH);
                    ids.into_iter().skip(skip).collect()
                })
                .unwrap_or_default(),
            frame_id: text_field(raw, "frameId"),
            container_id: text_field(raw, "containerId"),
            start: raw
                .get("startBinding")
                .and_then(|b| text_field(b, "elementId")),
            end: raw
                .get("endBinding")
                .and_then(|b| text_field(b, "elementId")),
        },
    })
}

fn text_field(raw: &Value, key: &str) -> Option<String> {
    raw.get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn kind(raw: &Value) -> Option<ElementKind> {
    match raw.get("type").and_then(Value::as_str)? {
        "rectangle" => Some(ElementKind::Rectangle {
            rounded: !matches!(raw.get("roundness"), None | Some(Value::Null)),
        }),
        "ellipse" => Some(ElementKind::Ellipse),
        "diamond" => Some(ElementKind::Diamond),
        "frame" | "magicframe" => Some(ElementKind::Frame {
            name: raw
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .map(str::to_string),
        }),
        "line" => Some(ElementKind::Line {
            points: points(raw),
            start_arrow: arrowhead(raw, "startArrowhead", false),
            end_arrow: arrowhead(raw, "endArrowhead", false),
        }),
        "arrow" => Some(ElementKind::Arrow {
            points: points(raw),
            start_arrow: arrowhead(raw, "startArrowhead", false),
            // An arrow with no `endArrowhead` key at all is still an arrow.
            end_arrow: arrowhead(raw, "endArrowhead", true),
        }),
        "text" => Some(ElementKind::Text {
            text: raw
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            font_size: number(raw, "fontSize", 20.0).clamp(1.0, MAX_FONT_SIZE),
            family: match raw.get("fontFamily").and_then(Value::as_i64) {
                Some(1) => FontFamily::Hand,
                Some(3) => FontFamily::Code,
                _ => FontFamily::Normal,
            },
            align: match raw.get("textAlign").and_then(Value::as_str) {
                Some("center") => TextAlign::Center,
                Some("right") => TextAlign::Right,
                _ => TextAlign::Left,
            },
        }),
        "freedraw" => Some(ElementKind::FreeDraw {
            points: points(raw),
        }),
        "image" => Some(ElementKind::Image {
            file_id: raw.get("fileId").and_then(Value::as_str)?.to_string(),
        }),
        _ => None,
    }
}

/// `points` are `[dx, dy]` pairs **relative to the element's `x, y`**, and stay that way.
fn points(raw: &Value) -> Vec<(f32, f32)> {
    raw.get("points")
        .and_then(Value::as_array)
        .map(|pairs| {
            pairs
                .iter()
                .filter_map(|pair| {
                    let pair = pair.as_array()?;
                    Some((
                        finite(pair.first()?.as_f64()?)?,
                        finite(pair.get(1)?.as_f64()?)?,
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// An arrowhead is present when the key holds a name other than `"none"`. An absent key falls back
/// to the type's own default; an explicit `null` does not.
fn arrowhead(raw: &Value, key: &str, absent: bool) -> bool {
    match raw.get(key) {
        None => absent,
        Some(Value::String(name)) => name != "none",
        _ => false,
    }
}

fn number(raw: &Value, key: &str, default: f32) -> f32 {
    raw.get(key)
        .and_then(Value::as_f64)
        .and_then(finite)
        .unwrap_or(default)
}

/// A file's number as a drawing coordinate: `None` unless finite, clamped to [`MAX_COORD`] so
/// `1e300` does not become an infinity in `f32`.
fn finite(n: f64) -> Option<f32> {
    n.is_finite()
        .then(|| n.clamp(-f64::from(MAX_COORD), f64::from(MAX_COORD)) as f32)
}

/// `default` is what an **absent** key means. A present `null`, `"transparent"` or an unknown name
/// is `None`, which is transparent.
fn colour(raw: &Value, key: &str, default: Option<Rgba8>) -> Option<Rgba8> {
    match raw.get(key) {
        None => default,
        Some(Value::String(text)) => Rgba8::parse(text),
        _ => None,
    }
}

fn embedded_files(value: &Value) -> HashMap<String, EmbeddedFile> {
    let Some(files) = value.get("files").and_then(Value::as_object) else {
        return HashMap::new();
    };
    files
        .iter()
        .filter_map(|(id, entry)| {
            let url = entry.get("dataURL").and_then(Value::as_str)?;
            let (mime, bytes) = codec::data_uri(url)?;
            Some((id.clone(), EmbeddedFile { mime, bytes }))
        })
        .collect()
}

/// The drawing out of an `.excalidraw.md` file: the fenced block under `## Drawing`.
///
/// The heading is what disambiguates, because these files are prose with a scene buried in them and
/// a document may well quote JSON somewhere else. A fence directly under `## Drawing` is the scene,
/// whatever else the file contains; a `json` fence anywhere else is only a fallback, taken when the
/// file has no such heading at all. A `compressed-json` fence is Obsidian's packed form and is
/// reported as [`SceneError::Compressed`] rather than guessed at.
fn drawing_fence(text: &str) -> Result<String, SceneError> {
    let mut under_drawing = false;
    let mut compressed = false;
    let mut fallback: Option<String> = None;

    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();

        // Any heading ends the previous section, so only a fence that follows `## Drawing`
        // immediately — with no heading in between — counts as the drawing.
        if trimmed.starts_with('#') {
            under_drawing = trimmed
                .trim_start_matches('#')
                .trim()
                .eq_ignore_ascii_case("drawing");
            continue;
        }

        let Some(lang) = trimmed.strip_prefix("```") else {
            continue;
        };
        let lang = lang.trim().to_ascii_lowercase();

        // Take the body whatever the language is: an unread fence still has to be stepped over,
        // or its contents would be scanned as if they were the document.
        let mut body = String::new();
        for body_line in lines.by_ref() {
            if body_line.trim_start().starts_with("```") {
                break;
            }
            body.push_str(body_line);
            body.push('\n');
        }

        match lang.as_str() {
            "compressed-json" if under_drawing => return Err(SceneError::Compressed),
            "compressed-json" => compressed = true,
            "json" | "excalidraw" if under_drawing => return Ok(body),
            "json" | "excalidraw" if fallback.is_none() => fallback = Some(body),
            _ => {}
        }
    }

    match fallback {
        Some(body) => Ok(body),
        None if compressed => Err(SceneError::Compressed),
        None => Err(SceneError::NotAScene),
    }
}
