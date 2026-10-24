//! An Excalidraw file, parsed — the element list the scene painter and the image editor share.
//!
//! **The parser lives in `crates/ubiq-draw`** (`ubiq_draw::excalidraw`), beside the draw.io
//! converter and the format-agnostic model both map onto. This module re-exports it under the
//! path the interface has always named, so the image editor (`state/image_edit.rs`), which builds
//! and edits a [`Scene`] of its own, and the tests keep one name for it.
//!
//! The rules — Excalidraw's stacking order with groups, frames and bound labels kept together,
//! unknown types skipped, `None` meaning transparent — are stated once, in that module's header.

pub use ubiq_draw::excalidraw::{
    Binding, Bounds, CORNER, Element, ElementKind, EmbeddedFile, FontFamily, Rgba8, Scene,
    SceneError, StrokeStyle, TextAlign, stacking_order,
};
