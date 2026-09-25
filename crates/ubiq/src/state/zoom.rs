//! The image/diagram zoom modal's own record (T-185).
//!
//! A picture drawn small in a document — a markdown image or a Mermaid diagram, both scaled down
//! to the reading measure — carries a corner button that opens the same picture near-fullscreen,
//! on the same pan-and-zoom camera [`crate::state::viewport`] already gives a panel. One record,
//! because only one instance is ever up at a time, on [`crate::state::overlay::Layer`]'s own rule.

use std::sync::Arc;

use gpui::Image;

/// One picture raised into the zoom modal.
#[derive(Clone)]
pub struct ImageZoom {
    /// The camera key this picture's pan and zoom are kept under — its own
    /// [`crate::state::viewport::Viewport`] entry, distinct from the small inline one so opening
    /// the modal does not fight the document's own camera over the same state.
    pub key: String,
    /// What the modal's header says this picture is — the fence's language, or the image's own alt
    /// text or file name.
    pub title: String,
    pub image: Arc<Image>,
    pub width: f32,
    pub height: f32,
}
