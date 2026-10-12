//! One mission, drawn.
//!
//! Two surfaces over one mission (`_docs/wip/note-plan.md`): the **side panel** in
//! [`panel`] — a fast overview, short, no scrolling in the ordinary case — and the **full view**
//! in [`full`], which is one view in two shapes, a modal and a document tab. They share
//! [`crate::state::mission::MissionView`], so the panel's progress bar hands the full view a
//! filter without either surface owning the other's state, and `Open as tab` keeps the reader
//! where they were. [`menu`] is the panel's `⋯`.
//!
//! [`wbs`] is the full view's WBS tab and [`settings`] its Settings tab, each a file of its own
//! because [`full`] is the frame the tabs hang off rather than the place they are all written.
//!
//! Nothing here invents data: everything drawn is the parent task, its subtasks and the host's
//! `MissionSnapshot`. [`note`] is the one dialog every "say why" gesture goes through.

pub mod full;
pub mod menu;
pub mod note;
pub mod panel;
pub mod settings;
pub mod wbs;

pub use panel::render;
