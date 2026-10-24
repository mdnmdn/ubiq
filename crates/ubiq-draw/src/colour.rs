//! A colour a file named, in channels.
//!
//! **A colour here is content, not design.** It is read out of a document the way a photograph's
//! pixels are, and the painter passes it through; nothing in this crate picks a colour for the
//! interface.

/// A straight 8-bit colour. Both formats write CSS-style colours; the renderer wants channels.
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

    /// Excalidraw's default stroke, and the fallback for an absent `strokeColor`.
    pub const DEFAULT_STROKE: Self = Self::opaque(0x1e, 0x1e, 0x1e);

    /// The paper both formats draw on when a file names no canvas colour. White is the file's own
    /// ground, not a theme token: a drawing is paper, and the panel behind it follows the file
    /// rather than the window.
    pub const DEFAULT_BACKGROUND: Self = Self::opaque(0xff, 0xff, 0xff);

    /// draw.io's default stroke, font colour and edge colour.
    pub const BLACK: Self = Self::opaque(0, 0, 0);

    /// draw.io's default vertex fill.
    pub const WHITE: Self = Self::opaque(0xff, 0xff, 0xff);

    /// Parse one CSS colour.
    ///
    /// `None` is transparent — that covers `"transparent"`, `"none"`, the empty string, and a name
    /// this module does not know. **An unrecognised name is transparent rather than an error**: a
    /// drawing with one exotic colour in it still draws.
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
