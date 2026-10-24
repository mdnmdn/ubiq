//! The drawing engine: one model for a 2-D diagram, and a converter onto it per file format.
//!
//! **Pure Rust — no GPUI type, no Ubiq crate.** The interface paints a [`Drawing`]; this crate only
//! turns bytes into one, which is what lets every rule about a format be asserted in a test with no
//! window.
//!
//! - [`model`] is the representation: shapes in a group tree that **is** the paint order, each
//!   with its own stroke, fill, opacity, rotation and an optional text box, plus the extent and the
//!   canvas colour. Colours are data ([`Rgba8`]), never a theme decision.
//! - [`excalidraw`] parses a `.excalidraw` file into its own element list (which the image editor
//!   also edits) and maps that onto the model — [`excalidraw::Scene::to_drawing`].
//! - [`drawio`] parses draw.io's mxGraph XML — plain, compressed, multi-page, or embedded in a
//!   `.drawio.svg` — straight onto the model.
//!
//! **Nothing here fails on one odd element.** An unknown Excalidraw type is skipped; an unknown
//! draw.io shape becomes a [`model::Geometry::Placeholder`] carrying its label. A whole-file error
//! is only ever "this is not a drawing at all".

pub mod colour;
pub mod drawio;
pub mod excalidraw;
pub mod model;

mod codec;

pub use colour::Rgba8;
pub use model::{
    ArrowHead, Bounds, Connector, Drawing, EmbeddedImage, FontFamily, Geometry, Group, Node, Point,
    Rect, Shape, StrokeStyle, Style, TextAlign, TextBox, VAlign,
};
