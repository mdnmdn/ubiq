//! A draw.io file — mxGraph XML — mapped onto the drawing model.
//!
//! Every container draw.io writes is read: a bare `<mxGraphModel>`, an `<mxfile>` of one or more
//! `<diagram>` pages with the model inline **or compressed** (base64 of raw deflate of the
//! URI-encoded XML), and a `.drawio.svg` whose root `<svg>` carries the `<mxfile>` in its
//! `content` attribute. [`parse`] draws the first page; [`parse_page`] any other, by index, and
//! [`pages`] names them.
//!
//! **The cell order is the paint order**, per parent: a layer is a [`Group`], a vertex with
//! children is a [`Group`] whose first child is the vertex itself, and a child's geometry is
//! relative to its parent vertex. An edge is routed here — straight through its waypoints, or
//! orthogonally for an `edgeStyle` that says so — and clipped to the shapes it joins.
//!
//! **What is not understood is drawn as a placeholder, never dropped and never fatal.** A shape
//! name outside the common set (`cloud`, `cylinder3`, a stencil, …) becomes
//! [`Geometry::Placeholder`] with its label, or its shape name when it has none.

use std::collections::{HashMap, HashSet};

use crate::codec;
use crate::colour::Rgba8;
use crate::model::{
    ArrowHead, Bounds, Connector, Drawing, EmbeddedImage, FontFamily, Geometry, Group, Node, Point,
    Rect, Shape, StrokeStyle, Style, TextAlign, TextBox, VAlign,
};
use crate::model::{MAX_COORD, MAX_FONT_SIZE};

/// draw.io's default label size.
const FONT_SIZE: f32 = 12.0;
/// mxGraph's `RECTANGLE_ROUNDING_FACTOR`, as a percentage: a rounded box's default `arcSize`.
const ARC_SIZE: f32 = 15.0;
/// A parent chain longer than this is a malformed file, not a diagram.
const MAX_DEPTH: usize = 64;
/// The largest a compressed page body may be before it is decoded at all.
const MAX_PACKED: usize = 16 * 1024 * 1024;

/// Why a file did not become a drawing. One odd cell is never one of these.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DrawioError {
    NotUtf8,
    /// The XML did not parse.
    Xml(String),
    /// Well-formed XML with no mxGraph model in it.
    NotADiagram,
    /// The page asked for is not in the file.
    NoSuchPage(usize),
    /// A compressed page that would not inflate.
    Undecodable,
}

impl std::fmt::Display for DrawioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DrawioError::NotUtf8 => write!(f, "this file is not text"),
            DrawioError::Xml(why) => write!(f, "this diagram did not parse: {why}"),
            DrawioError::NotADiagram => write!(f, "this file holds no draw.io diagram"),
            DrawioError::NoSuchPage(n) => write!(f, "this diagram has no page {}", n + 1),
            DrawioError::Undecodable => write!(f, "this diagram's page could not be decompressed"),
        }
    }
}

impl std::error::Error for DrawioError {}

/// One page of a file, as the file names it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Page {
    pub id: String,
    pub name: String,
}

/// The first page, drawn.
pub fn parse(bytes: &[u8]) -> Result<Drawing, DrawioError> {
    parse_page(bytes, 0)
}

/// Page `index` (from 0), drawn.
pub fn parse_page(bytes: &[u8], index: usize) -> Result<Drawing, DrawioError> {
    let file = mxfile_text(bytes)?;
    let doc = xml(&file)?;
    let root = doc.root_element();
    let model = if root.has_tag_name("mxGraphModel") {
        if index != 0 {
            return Err(DrawioError::NoSuchPage(index));
        }
        file[root.range()].to_string()
    } else {
        let diagram = root
            .descendants()
            .filter(|n| n.has_tag_name("diagram"))
            .nth(index)
            .ok_or(if index == 0 {
                DrawioError::NotADiagram
            } else {
                DrawioError::NoSuchPage(index)
            })?;
        match diagram.children().find(|n| n.has_tag_name("mxGraphModel")) {
            Some(inline) => file[inline.range()].to_string(),
            None => {
                let packed: String = diagram
                    .children()
                    .filter(|n| n.is_text())
                    .filter_map(|n| n.text())
                    .collect();
                decode_diagram(&packed).ok_or(DrawioError::Undecodable)?
            }
        }
    };
    convert_model(&model)
}

/// The pages of a file, in order. A bare model is one unnamed page.
pub fn pages(bytes: &[u8]) -> Result<Vec<Page>, DrawioError> {
    let file = mxfile_text(bytes)?;
    let doc = xml(&file)?;
    let root = doc.root_element();
    if root.has_tag_name("mxGraphModel") {
        return Ok(vec![Page {
            id: String::new(),
            name: String::new(),
        }]);
    }
    let found: Vec<Page> = root
        .descendants()
        .filter(|n| n.has_tag_name("diagram"))
        .map(|n| Page {
            id: n.attribute("id").unwrap_or_default().to_string(),
            name: n.attribute("name").unwrap_or_default().to_string(),
        })
        .collect();
    if found.is_empty() {
        return Err(DrawioError::NotADiagram);
    }
    Ok(found)
}

/// A compressed page body back to its model XML: base64, then raw inflate, then URI-decode.
/// `None` when any step fails.
pub fn decode_diagram(packed: &str) -> Option<String> {
    let packed = packed.trim();
    if packed.starts_with('<') {
        return Some(packed.to_string());
    }
    if packed.len() > MAX_PACKED {
        return None;
    }
    let deflated = codec::base64(packed)?;
    let inflated = codec::inflate_raw(&deflated)?;
    let text = std::str::from_utf8(&inflated).ok()?;
    Some(codec::percent_decode(text))
}

/// The `<mxfile>` (or bare model) text, unwrapping a `.drawio.svg`.
fn mxfile_text(bytes: &[u8]) -> Result<String, DrawioError> {
    let text = std::str::from_utf8(bytes).map_err(|_| DrawioError::NotUtf8)?;
    let text = text.trim_start_matches('\u{feff}');
    let doc = xml(text)?;
    let root = doc.root_element();
    if root.has_tag_name("svg") {
        return root
            .attribute("content")
            .map(str::to_string)
            .ok_or(DrawioError::NotADiagram);
    }
    Ok(text.to_string())
}

fn xml(text: &str) -> Result<roxmltree::Document<'_>, DrawioError> {
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    roxmltree::Document::parse_with_options(text, options)
        .map_err(|why| DrawioError::Xml(why.to_string()))
}

// ------------------------------------------------------------------------------------------- //
// The model
// ------------------------------------------------------------------------------------------- //

#[derive(Default, Clone)]
struct Geo {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    relative: bool,
    points: Vec<Point>,
    source_point: Option<Point>,
    target_point: Option<Point>,
}

struct Cell {
    id: String,
    parent: Option<String>,
    value: String,
    style: Styles,
    vertex: bool,
    edge: bool,
    visible: bool,
    source: Option<String>,
    target: Option<String>,
    geo: Geo,
}

/// A parsed style string: `key=value` pairs, plus the bare names (`ellipse`, `text`, `group`).
#[derive(Default, Clone)]
struct Styles {
    pairs: HashMap<String, String>,
    /// The first bare name, which is the shape when no `shape=` says otherwise.
    base: Option<String>,
}

impl Styles {
    fn parse(style: &str) -> Self {
        let mut out = Styles::default();
        let mut tokens = style.split(';').peekable();
        while let Some(token) = tokens.next() {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            match token.split_once('=') {
                Some((key, value)) => {
                    let mut value = value.to_string();
                    // `image=data:image/png;base64,…` splits on its own `;`.
                    if key == "image"
                        && value.starts_with("data:")
                        && tokens
                            .peek()
                            .is_some_and(|next| next.starts_with("base64,"))
                    {
                        value.push(';');
                        value.push_str(tokens.next().unwrap_or_default());
                    }
                    out.pairs.insert(key.to_string(), value);
                }
                None => {
                    out.pairs.insert(token.to_string(), "1".to_string());
                    if out.base.is_none() {
                        out.base = Some(token.to_string());
                    }
                }
            }
        }
        out
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.pairs.get(key).map(String::as_str)
    }

    fn flag(&self, key: &str) -> bool {
        self.get(key).is_some_and(|v| v == "1" || v == "true")
    }

    fn number(&self, key: &str) -> Option<f32> {
        self.get(key)
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(-MAX_COORD, MAX_COORD))
    }

    /// `fontSize`, defaulted and clamped to something a label can be drawn at.
    fn font_size(&self) -> f32 {
        self.number("fontSize")
            .unwrap_or(FONT_SIZE)
            .clamp(1.0, MAX_FONT_SIZE)
    }

    /// The shape name: `shape=` when present, else the first bare name, else `rect`.
    fn shape(&self) -> &str {
        self.get("shape").or(self.base.as_deref()).unwrap_or("rect")
    }

    /// A colour key: absent or `default` is `default`; `none` is transparent.
    fn colour(&self, key: &str, default: Option<Rgba8>) -> Option<Rgba8> {
        match self.get(key) {
            None | Some("default") | Some("inherit") => default,
            Some(value) => Rgba8::parse(value),
        }
    }
}

fn attr_f32(node: roxmltree::Node<'_, '_>, key: &str) -> f32 {
    node.attribute(key)
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .map(|v| v.clamp(-MAX_COORD, MAX_COORD))
        .unwrap_or(0.0)
}

fn point_of(node: roxmltree::Node<'_, '_>) -> Point {
    Point::new(attr_f32(node, "x"), attr_f32(node, "y"))
}

fn geometry(node: roxmltree::Node<'_, '_>) -> Geo {
    let Some(geo) = node.children().find(|n| n.has_tag_name("mxGeometry")) else {
        return Geo::default();
    };
    let mut out = Geo {
        x: attr_f32(geo, "x"),
        y: attr_f32(geo, "y"),
        w: attr_f32(geo, "width"),
        h: attr_f32(geo, "height"),
        relative: geo.attribute("relative") == Some("1"),
        ..Geo::default()
    };
    for child in geo.children().filter(|n| n.is_element()) {
        match (child.tag_name().name(), child.attribute("as")) {
            ("mxPoint", Some("sourcePoint")) => out.source_point = Some(point_of(child)),
            ("mxPoint", Some("targetPoint")) => out.target_point = Some(point_of(child)),
            ("Array", Some("points")) => {
                out.points = child
                    .children()
                    .filter(|n| n.has_tag_name("mxPoint"))
                    .map(point_of)
                    .collect();
            }
            _ => {}
        }
    }
    out
}

/// Every cell under `<root>`, in document order. An `<object>`/`<UserObject>` wrapper lends its
/// id and `label` to the `<mxCell>` inside it.
fn cells(model: roxmltree::Node<'_, '_>) -> Vec<Cell> {
    let Some(root) = model.children().find(|n| n.has_tag_name("root")) else {
        return Vec::new();
    };
    root.children()
        .filter(|n| n.is_element())
        .filter_map(|node| {
            let (holder, cell) = if node.has_tag_name("mxCell") {
                (node, node)
            } else {
                let inner = node.children().find(|n| n.has_tag_name("mxCell"))?;
                (node, inner)
            };
            let value = if holder == cell {
                cell.attribute("value")
            } else {
                holder.attribute("label")
            };
            Some(Cell {
                id: holder.attribute("id").unwrap_or_default().to_string(),
                parent: cell.attribute("parent").map(str::to_string),
                value: value.unwrap_or_default().to_string(),
                style: Styles::parse(cell.attribute("style").unwrap_or_default()),
                vertex: cell.attribute("vertex") == Some("1"),
                edge: cell.attribute("edge") == Some("1"),
                visible: cell.attribute("visible") != Some("0"),
                source: cell.attribute("source").map(str::to_string),
                target: cell.attribute("target").map(str::to_string),
                geo: geometry(cell),
            })
        })
        .collect()
}

struct Graph {
    cells: Vec<Cell>,
    by_id: HashMap<String, usize>,
    children: HashMap<String, Vec<usize>>,
    images: HashMap<String, EmbeddedImage>,
}

impl Graph {
    fn cell(&self, id: &str) -> Option<&Cell> {
        self.by_id.get(id).map(|&i| &self.cells[i])
    }

    /// The absolute origin a cell's own geometry is relative to: the sum of its vertex ancestors'
    /// positions. A layer or the root contributes nothing.
    fn origin(&self, cell: &Cell) -> Point {
        let mut at = Point::default();
        let mut parent = cell.parent.as_deref();
        for _ in 0..MAX_DEPTH {
            let Some(p) = parent.and_then(|id| self.cell(id)) else {
                break;
            };
            if p.vertex && !p.geo.relative {
                at.x += p.geo.x;
                at.y += p.geo.y;
            }
            parent = p.parent.as_deref();
        }
        at
    }

    /// A vertex's absolute box.
    fn rect(&self, cell: &Cell) -> Rect {
        let o = self.origin(cell);
        Rect::new(o.x + cell.geo.x, o.y + cell.geo.y, cell.geo.w, cell.geo.h)
    }
}

fn convert_model(text: &str) -> Result<Drawing, DrawioError> {
    let doc = xml(text)?;
    let model = doc
        .descendants()
        .find(|n| n.has_tag_name("mxGraphModel"))
        .ok_or(DrawioError::NotADiagram)?;
    let background = model.attribute("background").and_then(Rgba8::parse);

    let cells = cells(model);
    let mut by_id = HashMap::new();
    let mut children: HashMap<String, Vec<usize>> = HashMap::new();
    let mut roots = Vec::new();
    for (i, cell) in cells.iter().enumerate() {
        // A repeated id is malformed: the first cell keeps it and the later ones are dropped,
        // or each copy would build the whole subtree again and the walk would grow
        // exponentially with the number of repeats.
        if *by_id.entry(cell.id.clone()).or_insert(i) != i {
            continue;
        }
        match &cell.parent {
            Some(parent) if *parent != cell.id => {
                children.entry(parent.clone()).or_default().push(i)
            }
            Some(_) => {}
            None => roots.push(i),
        }
    }
    let mut graph = Graph {
        cells,
        by_id,
        children,
        images: HashMap::new(),
    };

    let mut root = Group::default();
    let mut visited = HashSet::new();
    for i in roots {
        let id = graph.cells[i].id.clone();
        if visited.insert(i) {
            root.children
                .extend(build(&mut graph, &id, 0, &mut visited));
        }
    }

    let mut drawing = Drawing {
        root,
        background,
        images: std::mem::take(&mut graph.images),
        bounds: Bounds::EMPTY,
    };
    drawing.bounds =
        Bounds::of(drawing.shapes().into_iter().flat_map(|shape| {
            std::iter::once(shape.frame).chain(shape.text.as_ref().map(|t| t.rect))
        }));
    Ok(drawing)
}

/// The nodes under one cell, in its children's order. `visited` holds every cell index built so
/// far, so no cell is drawn twice whatever the file's parent links say.
fn build(graph: &mut Graph, parent: &str, depth: usize, visited: &mut HashSet<usize>) -> Vec<Node> {
    if depth > MAX_DEPTH {
        return Vec::new();
    }
    let kids = graph.children.get(parent).cloned().unwrap_or_default();
    let mut out = Vec::with_capacity(kids.len());
    for i in kids {
        if !graph.cells[i].visible || !visited.insert(i) {
            continue;
        }
        let id = graph.cells[i].id.clone();
        let has_children = graph.children.get(&id).is_some_and(|k| !k.is_empty());
        let (vertex, edge) = (graph.cells[i].vertex, graph.cells[i].edge);
        let own: Option<Shape> = if edge {
            edge_shape(graph, i)
        } else if vertex {
            vertex_shape(graph, i)
        } else {
            None
        };
        if has_children {
            let mut group = Group {
                id: id.clone(),
                children: own.into_iter().map(Node::Shape).collect(),
            };
            group.children.extend(build(graph, &id, depth + 1, visited));
            out.push(Node::Group(group));
        } else if let Some(shape) = own {
            out.push(Node::Shape(shape));
        }
    }
    out
}

// ------------------------------------------------------------------------------------------- //
// Vertices
// ------------------------------------------------------------------------------------------- //

fn ink(styles: &Styles, stroke: Option<Rgba8>, fill: Option<Rgba8>) -> Style {
    Style {
        stroke: styles.colour("strokeColor", stroke),
        fill: styles.colour("fillColor", fill),
        stroke_width: styles.number("strokeWidth").unwrap_or(1.0).max(0.0),
        stroke_style: if styles.flag("dashed") {
            StrokeStyle::Dashed
        } else {
            StrokeStyle::Solid
        },
        opacity: (styles.number("opacity").unwrap_or(100.0) / 100.0).clamp(0.0, 1.0),
    }
}

/// A label as plain text: `html=1` labels lose their markup, keeping their line breaks.
fn label_text(cell: &Cell) -> String {
    if cell.style.flag("html") {
        html_to_text(&cell.value)
    } else {
        cell.value.clone()
    }
}

fn text_box(styles: &Styles, text: String, rect: Rect, valign: VAlign) -> Option<TextBox> {
    if text.trim().is_empty() {
        return None;
    }
    let family = match styles.get("fontFamily") {
        Some(f) if f.to_ascii_lowercase().contains("courier") || f.contains("mono") => {
            FontFamily::Code
        }
        _ => FontFamily::Normal,
    };
    Some(TextBox {
        text,
        rect,
        font_size: styles.font_size(),
        family,
        align: match styles.get("align") {
            Some("left") => TextAlign::Left,
            Some("right") => TextAlign::Right,
            _ => TextAlign::Center,
        },
        valign: match styles.get("verticalAlign") {
            Some("top") => VAlign::Top,
            Some("bottom") => VAlign::Bottom,
            Some("middle") => VAlign::Middle,
            _ => valign,
        },
        colour: styles.colour("fontColor", Some(Rgba8::BLACK)),
    })
}

fn vertex_shape(graph: &mut Graph, i: usize) -> Option<Shape> {
    let cell = &graph.cells[i];
    // A label on an edge is a child vertex with relative geometry: draw its text at the edge.
    if let Some(parent) = cell.parent.as_deref().and_then(|p| graph.cell(p))
        && parent.edge
    {
        let parent_index = graph.by_id[&parent.id];
        let points = edge_points(graph, parent_index);
        let at = midpoint(&points)?;
        let text = label_text(cell);
        let rect = label_rect(&text, cell.style.font_size(), at);
        return Some(Shape {
            id: cell.id.clone(),
            geometry: Geometry::Text,
            frame: rect,
            rotation: 0.0,
            style: ink(&cell.style, None, None),
            text: text_box(&cell.style, text, rect, VAlign::Middle),
        });
    }

    let rect = graph.rect(cell);
    let id = cell.id.clone();
    let styles = cell.style.clone();
    let name = styles.shape().to_string();
    let rotation = styles.number("rotation").unwrap_or(0.0).to_radians();
    let text = label_text(cell);
    let mut valign = VAlign::Middle;

    let (geometry, style) = match name.as_str() {
        "text" => (Geometry::Text, ink(&styles, None, None)),
        "group" => (Geometry::Rect, ink(&styles, None, None)),
        "ellipse" | "doubleEllipse" => (
            Geometry::Ellipse,
            ink(&styles, Some(Rgba8::BLACK), Some(Rgba8::WHITE)),
        ),
        "rhombus" => (
            Geometry::Diamond,
            ink(&styles, Some(Rgba8::BLACK), Some(Rgba8::WHITE)),
        ),
        "triangle" => (
            Geometry::Polyline {
                points: triangle(rect, styles.get("direction")),
                closed: true,
            },
            ink(&styles, Some(Rgba8::BLACK), Some(Rgba8::WHITE)),
        ),
        "hexagon" => (
            Geometry::Polyline {
                points: hexagon(rect),
                closed: true,
            },
            ink(&styles, Some(Rgba8::BLACK), Some(Rgba8::WHITE)),
        ),
        "line" => (
            Geometry::Polyline {
                points: vec![
                    Point::new(rect.x, rect.y + rect.h / 2.0),
                    Point::new(rect.x + rect.w, rect.y + rect.h / 2.0),
                ],
                closed: false,
            },
            ink(&styles, Some(Rgba8::BLACK), None),
        ),
        "image" => {
            let image = styles
                .get("image")
                .and_then(codec::data_uri)
                .map(|(mime, bytes)| {
                    graph
                        .images
                        .insert(id.clone(), EmbeddedImage { mime, bytes });
                    id.clone()
                });
            valign = VAlign::Top;
            (Geometry::Image { image }, ink(&styles, None, None))
        }
        "rect" | "label" | "process" | "swimlane" | "rectangle" => {
            if name == "swimlane" {
                valign = VAlign::Top;
            }
            let geometry = if styles.flag("rounded") {
                let arc = styles.number("arcSize").unwrap_or(ARC_SIZE);
                let radius = if styles.flag("absoluteArcSize") {
                    arc / 2.0
                } else {
                    rect.w.abs().min(rect.h.abs()) * arc / 100.0
                };
                Geometry::RoundedRect { radius }
            } else {
                Geometry::Rect
            };
            (
                geometry,
                ink(&styles, Some(Rgba8::BLACK), Some(Rgba8::WHITE)),
            )
        }
        _ => (
            Geometry::Placeholder,
            ink(&styles, Some(Rgba8::BLACK), None),
        ),
    };

    // A placeholder always says what it stands for.
    let text = if geometry == Geometry::Placeholder && text.trim().is_empty() {
        name.clone()
    } else {
        text
    };
    let text = text_box(&styles, text, rect, valign);
    if name == "group" && text.is_none() {
        // An invisible container: its children are the drawing.
        return None;
    }
    let geometry = if name == "group" {
        Geometry::Text
    } else {
        geometry
    };
    Some(Shape {
        id,
        geometry,
        frame: rect,
        rotation,
        style,
        text,
    })
}

fn triangle(r: Rect, direction: Option<&str>) -> Vec<Point> {
    let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    match direction {
        Some("north") => vec![Point::new(cx, y0), Point::new(x1, y1), Point::new(x0, y1)],
        Some("south") => vec![Point::new(x0, y0), Point::new(x1, y0), Point::new(cx, y1)],
        Some("west") => vec![Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, cy)],
        _ => vec![Point::new(x0, y0), Point::new(x1, cy), Point::new(x0, y1)],
    }
}

fn hexagon(r: Rect) -> Vec<Point> {
    let inset = r.w * 0.25;
    let cy = r.y + r.h / 2.0;
    vec![
        Point::new(r.x + inset, r.y),
        Point::new(r.x + r.w - inset, r.y),
        Point::new(r.x + r.w, cy),
        Point::new(r.x + r.w - inset, r.y + r.h),
        Point::new(r.x + inset, r.y + r.h),
        Point::new(r.x, cy),
    ]
}

// ------------------------------------------------------------------------------------------- //
// Edges
// ------------------------------------------------------------------------------------------- //

fn head(styles: &Styles, key: &str, fill_key: &str, default: &str) -> ArrowHead {
    let name = styles.get(key).unwrap_or(default);
    match name {
        "" | "none" => ArrowHead::None,
        "open" | "openThin" | "openAsync" => ArrowHead::Open,
        _ if styles.get(fill_key) == Some("0") => ArrowHead::Open,
        _ => ArrowHead::Filled,
    }
}

fn edge_shape(graph: &mut Graph, i: usize) -> Option<Shape> {
    let points = edge_points(graph, i);
    if points.len() < 2 {
        return None;
    }
    let cell = &graph.cells[i];
    let styles = &cell.style;
    let frame = rect_of(&points);
    let text = label_text(cell);
    let size = styles.font_size();
    let text = midpoint(&points).and_then(|at| {
        text_box(
            styles,
            text.clone(),
            label_rect(&text, size, at),
            VAlign::Middle,
        )
    });
    Some(Shape {
        id: cell.id.clone(),
        geometry: Geometry::Connector(Connector {
            points,
            start: head(styles, "startArrow", "startFill", "none"),
            end: head(styles, "endArrow", "endFill", "classic"),
            source: cell.source.clone(),
            target: cell.target.clone(),
        }),
        frame,
        rotation: 0.0,
        style: ink(styles, Some(Rgba8::BLACK), None),
        text,
    })
}

/// The route an edge draws: its two ends, clipped to the shapes they join, through its
/// waypoints — orthogonally when its `edgeStyle` asks for it.
fn edge_points(graph: &Graph, i: usize) -> Vec<Point> {
    let cell = &graph.cells[i];
    let origin = graph.origin(cell);
    let shift = |p: Point| Point::new(p.x + origin.x, p.y + origin.y);
    let waypoints: Vec<Point> = cell.geo.points.iter().copied().map(shift).collect();
    let terminal = |id: &Option<String>| {
        id.as_deref()
            .and_then(|id| graph.cell(id))
            .filter(|c| c.vertex)
            .map(|c| (graph.rect(c), c.style.shape() == "ellipse"))
    };
    let source = terminal(&cell.source);
    let target = terminal(&cell.target);
    let styles = &cell.style;
    let orthogonal = matches!(
        styles.get("edgeStyle"),
        Some(
            "orthogonalEdgeStyle"
                | "elbowEdgeStyle"
                | "entityRelationEdgeStyle"
                | "segmentEdgeStyle"
                | "isometricEdgeStyle"
        )
    );

    // A fixed port (`exitX`/`entryX`) is a point on the box; otherwise the box's centre, clipped
    // later toward whatever the route meets next.
    let port = |rect: Rect, kx: &str, ky: &str| -> Option<Point> {
        let (fx, fy) = (styles.number(kx)?, styles.number(ky)?);
        Some(Point::new(rect.x + rect.w * fx, rect.y + rect.h * fy))
    };
    let start_fixed = source.and_then(|(r, _)| port(r, "exitX", "exitY"));
    let end_fixed = target.and_then(|(r, _)| port(r, "entryX", "entryY"));

    let start_raw = start_fixed
        .or(source.map(|(r, _)| r.centre()))
        .or(cell.geo.source_point.map(shift));
    let end_raw = end_fixed
        .or(target.map(|(r, _)| r.centre()))
        .or(cell.geo.target_point.map(shift));
    let (Some(start_raw), Some(end_raw)) = (start_raw, end_raw) else {
        return Vec::new();
    };

    if orthogonal {
        return orthogonal_route(
            source.map(|(r, _)| r),
            target.map(|(r, _)| r),
            start_fixed.or(if source.is_none() {
                Some(start_raw)
            } else {
                None
            }),
            end_fixed.or(if target.is_none() {
                Some(end_raw)
            } else {
                None
            }),
            &waypoints,
        );
    }

    let toward_start = waypoints.first().copied().unwrap_or(end_raw);
    let toward_end = waypoints.last().copied().unwrap_or(start_raw);
    let start = match (start_fixed, source) {
        (None, Some((rect, ellipse))) => clip(rect, ellipse, toward_start),
        _ => start_raw,
    };
    let end = match (end_fixed, target) {
        (None, Some((rect, ellipse))) => clip(rect, ellipse, toward_end),
        _ => end_raw,
    };
    let mut points = Vec::with_capacity(waypoints.len() + 2);
    points.push(start);
    points.extend(waypoints);
    points.push(end);
    points
}

/// Where the ray from `rect`'s centre toward `toward` leaves the box (or the ellipse inside it).
fn clip(rect: Rect, ellipse: bool, toward: Point) -> Point {
    let c = rect.centre();
    let (dx, dy) = (toward.x - c.x, toward.y - c.y);
    let (hw, hh) = (rect.w.abs() / 2.0, rect.h.abs() / 2.0);
    if (dx == 0.0 && dy == 0.0) || hw == 0.0 || hh == 0.0 {
        return c;
    }
    let t = if ellipse {
        1.0 / ((dx / hw).powi(2) + (dy / hh).powi(2)).sqrt()
    } else {
        let tx = if dx != 0.0 {
            hw / dx.abs()
        } else {
            f32::INFINITY
        };
        let ty = if dy != 0.0 {
            hh / dy.abs()
        } else {
            f32::INFINITY
        };
        tx.min(ty)
    };
    let t = t.min(1.0);
    Point::new(c.x + dx * t, c.y + dy * t)
}

/// An elbow route: out of the source's facing side, through the waypoints with a corner between
/// any two that are not aligned, into the target's facing side.
fn orthogonal_route(
    source: Option<Rect>,
    target: Option<Rect>,
    start_fixed: Option<Point>,
    end_fixed: Option<Point>,
    waypoints: &[Point],
) -> Vec<Point> {
    let centre = |r: Option<Rect>, p: Option<Point>| p.or(r.map(|r| r.centre()));
    let (Some(sc), Some(ec)) = (centre(source, start_fixed), centre(target, end_fixed)) else {
        return Vec::new();
    };
    let toward_start = waypoints.first().copied().unwrap_or(ec);
    let toward_end = waypoints.last().copied().unwrap_or(sc);
    let start = start_fixed.unwrap_or_else(|| side(source, toward_start));
    let end = end_fixed.unwrap_or_else(|| side(target, toward_end));

    let mut route = vec![start];
    route.extend_from_slice(waypoints);
    route.push(end);

    let mut out: Vec<Point> = Vec::with_capacity(route.len() * 2);
    out.push(route[0]);
    for pair in route.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if (a.x - b.x).abs() > 0.5 && (a.y - b.y).abs() > 0.5 {
            if waypoints.is_empty() {
                // No waypoints: one dog-leg through the middle, along the longer axis.
                if (b.x - a.x).abs() >= (b.y - a.y).abs() {
                    let mx = (a.x + b.x) / 2.0;
                    out.push(Point::new(mx, a.y));
                    out.push(Point::new(mx, b.y));
                } else {
                    let my = (a.y + b.y) / 2.0;
                    out.push(Point::new(a.x, my));
                    out.push(Point::new(b.x, my));
                }
            } else {
                out.push(Point::new(b.x, a.y));
            }
        }
        out.push(b);
    }
    out
}

/// The midpoint of the box side facing `toward`.
fn side(rect: Option<Rect>, toward: Point) -> Point {
    let Some(r) = rect else {
        return toward;
    };
    let c = r.centre();
    let (dx, dy) = (toward.x - c.x, toward.y - c.y);
    let (hw, hh) = (r.w.abs() / 2.0, r.h.abs() / 2.0);
    if dx.abs() * hh >= dy.abs() * hw {
        Point::new(c.x + hw.copysign(dx), c.y)
    } else {
        Point::new(c.x, c.y + hh.copysign(dy))
    }
}

fn rect_of(points: &[Point]) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in points {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    if x0 > x1 {
        return Rect::default();
    }
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// The point halfway along a polyline, by length.
fn midpoint(points: &[Point]) -> Option<Point> {
    let first = *points.first()?;
    let total: f32 = points
        .windows(2)
        .map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y))
        .sum();
    let mut left = total / 2.0;
    for w in points.windows(2) {
        let len = (w[1].x - w[0].x).hypot(w[1].y - w[0].y);
        if len >= left && len > 0.0 {
            let t = left / len;
            return Some(Point::new(
                w[0].x + (w[1].x - w[0].x) * t,
                w[0].y + (w[1].y - w[0].y) * t,
            ));
        }
        left -= len;
    }
    Some(first)
}

/// A box roughly the size of `text`, centred on `at`. The painter centres within it.
fn label_rect(text: &str, size: f32, at: Point) -> Rect {
    let lines = text.lines().count().max(1) as f32;
    let widest = text.lines().map(|l| l.chars().count()).max().unwrap_or(0) as f32;
    let (w, h) = (widest * size * 0.6 + 4.0, lines * size * 1.25);
    Rect::new(at.x - w / 2.0, at.y - h / 2.0, w, h)
}

// ------------------------------------------------------------------------------------------- //
// Labels
// ------------------------------------------------------------------------------------------- //

/// An `html=1` label as plain text. Block tags and `<br>` break the line; every other tag goes;
/// source whitespace collapses the way HTML collapses it; entities are decoded.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    let newline = |out: &mut String| {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
    };
    while let Some(open) = rest.find('<') {
        push_collapsed(&mut out, &rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            push_collapsed(&mut out, &rest[open..]);
            rest = "";
            break;
        };
        let tag = &rest[open + 1..open + close];
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        match name.as_str() {
            "br" => out.push('\n'),
            "div" | "p" | "li" | "tr" | "ul" | "ol" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            | "table" => newline(&mut out),
            _ => {}
        }
        rest = &rest[open + close + 1..];
    }
    push_collapsed(&mut out, rest);
    let decoded = decode_entities(&out);
    decoded
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_string()
}

fn push_collapsed(out: &mut String, text: &str) {
    for c in text.chars() {
        if c.is_whitespace() {
            if !out.ends_with(' ') && !out.ends_with('\n') && !out.is_empty() {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let decoded = tail.find(';').filter(|&end| end <= 10).and_then(|end| {
            let name = &tail[1..end];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            }?;
            Some((c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}
