//! An Excalidraw file, parsed into something a painter can walk without asking a question.
//!
//! A `.excalidraw` file is JSON with a closed vocabulary — not a language — so the viewer needs no
//! renderer in another process: it needs the file turned into shapes. That is all this module is.
//! **Nothing here draws and nothing here names a frame**, exactly as in
//! [`super::explorer`] and [`super::dock`], which is what lets `tests/scene.rs` assert paint order
//! and colour parsing without a window.
//!
//! **The subset is `_tools/excalidraw.py`'s**, deliberately. That tool already renders these files
//! to clean SVG for the documentation, `_docs/tech/diagram-format.md` states what it does and does
//! not reproduce, and `_docs/design/` is the corpus both are tested against. Reproducing a second,
//! larger subset here would mean two answers to "what does a diagram look like".
//!
//! Three rules the file format does not make obvious:
//!
//! - **Paint order is by type, not by the file's `index`.** Frames sit under everything, then
//!   shapes, then connectors, then text. The sort happens here, once, so the renderer walks
//!   [`Scene::elements`] front to back and stops thinking about it.
//! - **An element this module does not know is skipped, and the rest of the scene parses.** One
//!   unknown type is a missing shape, never a blank panel.
//! - **`None` on a colour means transparent**, not "use a default". The defaults are applied while
//!   parsing, so an absent `strokeColor` arrives as Excalidraw's `#1e1e1e` and an explicit
//!   `"transparent"` arrives as `None`.

use std::borrow::Cow;
use std::collections::HashMap;

use serde_json::Value;

/// A straight 8-bit colour. Excalidraw writes CSS colours; the renderer wants channels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Excalidraw's default stroke, and this module's fallback for an absent `strokeColor`.
    pub const DEFAULT_STROKE: Self = Self::opaque(0x1e, 0x1e, 0x1e);

    /// Excalidraw's default canvas, and this module's fallback for an absent
    /// `viewBackgroundColor`. White is the file's own ground, not a theme token: a scene is paper,
    /// and the panel behind it follows the file rather than the window.
    pub const DEFAULT_BACKGROUND: Self = Self::opaque(0xff, 0xff, 0xff);

    /// Parse one CSS colour the way `_tools/excalidraw.py` normalises it.
    ///
    /// `None` is transparent — that covers `"transparent"`, `"none"`, the empty string, and a name
    /// this module does not know. **An unrecognised name is transparent rather than an error**: a
    /// scene with one exotic colour in it still draws.
    pub fn parse(value: &str) -> Option<Self> {
        let text = value.trim();
        let lower = text.to_ascii_lowercase();
        if matches!(lower.as_str(), "" | "transparent" | "none") {
            return None;
        }
        let hex = text.strip_prefix('#').unwrap_or(text);
        if hex.chars().all(|c| c.is_ascii_hexdigit()) {
            match hex.len() {
                3 => {
                    let (r, g, b) = (nybble(hex, 0)?, nybble(hex, 1)?, nybble(hex, 2)?);
                    return Some(Self::opaque(r * 17, g * 17, b * 17));
                }
                6 => {
                    return Some(Self::opaque(byte(hex, 0)?, byte(hex, 2)?, byte(hex, 4)?));
                }
                8 => {
                    return Some(Self {
                        r: byte(hex, 0)?,
                        g: byte(hex, 2)?,
                        b: byte(hex, 4)?,
                        a: byte(hex, 6)?,
                    });
                }
                _ => return None,
            }
        }
        named_colour(&lower)
    }
}

fn nybble(hex: &str, at: usize) -> Option<u8> {
    u8::from_str_radix(hex.get(at..at + 1)?, 16).ok()
}

fn byte(hex: &str, at: usize) -> Option<u8> {
    u8::from_str_radix(hex.get(at..at + 2)?, 16).ok()
}

/// The basic CSS names, which is what a hand-written diagram reaches for. Anything else is
/// transparent — see [`Rgba8::parse`].
fn named_colour(name: &str) -> Option<Rgba8> {
    let (r, g, b) = match name {
        "black" => (0x00, 0x00, 0x00),
        "silver" => (0xc0, 0xc0, 0xc0),
        "gray" | "grey" => (0x80, 0x80, 0x80),
        "white" => (0xff, 0xff, 0xff),
        "maroon" => (0x80, 0x00, 0x00),
        "red" => (0xff, 0x00, 0x00),
        "purple" => (0x80, 0x00, 0x80),
        "fuchsia" | "magenta" => (0xff, 0x00, 0xff),
        "green" => (0x00, 0x80, 0x00),
        "lime" => (0x00, 0xff, 0x00),
        "olive" => (0x80, 0x80, 0x00),
        "yellow" => (0xff, 0xff, 0x00),
        "navy" => (0x00, 0x00, 0x80),
        "blue" => (0x00, 0x00, 0xff),
        "teal" => (0x00, 0x80, 0x80),
        "aqua" | "cyan" => (0x00, 0xff, 0xff),
        "orange" => (0xff, 0xa5, 0x00),
        _ => return None,
    };
    Some(Rgba8::opaque(r, g, b))
}

/// How a stroke is dashed. The reference renderer's three, and no more.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum StrokeStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

/// Excalidraw's three font families, numbered `1`, `2`, `3` in the file.
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
    /// Where this kind sits in the stack: frames under everything, then shapes, then connectors,
    /// then text. Equal ranks keep file order, which is why [`Scene`] sorts stably.
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
}

/// An image the file carried inline, decoded out of its `data:` URI.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EmbeddedFile {
    /// `image/png`, `image/svg+xml`, … — whatever the URI declared.
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// The extent of the live elements, unpadded. The renderer adds its own margin.
///
/// It is the union of each element's `x, y, x + width, y + height` — so, as in the reference
/// renderer, a connector whose points reach outside its own box is not accounted for.
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

        let bounds = bounds_of(&elements);
        let mut elements = elements;
        // Stable, so elements of equal rank keep the order the file wrote them in.
        elements.sort_by_key(|element| element.kind.paint_rank());

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
}

fn bounds_of(elements: &[Element]) -> Bounds {
    let mut bounds: Option<Bounds> = None;
    for element in elements {
        let (x0, x1) = (element.x, element.x + element.width);
        let (y0, y1) = (element.y, element.y + element.height);
        let next = Bounds {
            min_x: x0.min(x1),
            min_y: y0.min(y1),
            max_x: x0.max(x1),
            max_y: y0.max(y1),
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
    })
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
            font_size: number(raw, "fontSize", 20.0),
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
                        pair.first()?.as_f64()? as f32,
                        pair.get(1)?.as_f64()? as f32,
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
        .map(|n| n as f32)
        .unwrap_or(default)
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
            let (mime, bytes) = data_uri(url)?;
            Some((id.clone(), EmbeddedFile { mime, bytes }))
        })
        .collect()
}

/// `data:image/png;base64,….` Anything else — a remote URL, a URI that is not base64 — is not a
/// file this scene carries, and the image simply has no bytes.
fn data_uri(url: &str) -> Option<(String, Vec<u8>)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    let meta = meta.strip_suffix(";base64")?;
    let mime = meta.split(';').next().unwrap_or_default();
    Some((mime.to_string(), base64(payload)?))
}

/// Standard and URL-safe base64, whitespace tolerated. Thirty lines is cheaper than a dependency.
fn base64(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for byte in input.bytes() {
        let sextet = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(sextet);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
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
