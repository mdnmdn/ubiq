//! The block-list markdown view: one `gpui::list()` row per root block of a `ubiq_md::Document`,
//! drawn by this crate rather than by the component library's `TextView`. Ported from the
//! markdown-viewer spike (mission T-298).
//!
//! - [`blocks`] — one root block, one row: the typographic scale, the measure, the rhythm.
//! - [`prose`] — the native text element a paragraph or heading is shaped and painted in.
//! - [`inline`] — a block's inline spans to text runs, for both [`prose`] and `StyledText`.
//! - [`search`] — find-in-document state and the per-row hit plumbing (the find bar is later).
//! - [`fences`] — mermaid / excalidraw fences and image blocks, through `ui/viewer`'s renderers.
//! - [`sync`] — source ↔ preview scroll linking and reparse splicing, by block index.
//! - [`structure`] — the navigator's flat list of places (the document's headings).
//! - [`view`] — the `MdView` entity: a document over a shared buffer, its rows, reparse, scroll link.
//! - [`blockedit`] — the in-place block editor `MdView` swaps a row for.
//! - [`minimap`] — the structural strip down the page's edge, and its drag.
//! - [`outline`] — the heading navigator's rows and its jump popover.
//! - [`annotation`] — the margins' annotation layer: count marker, action stack, selection bar.
//! - [`highlight`] — the left-margin highlight dot and the `●` colour menu.
//! - [`events`] — what an `MdView` tells its host.
//!
//! A markdown file tab (IDE or knowledge base) holds one `MdView` over its buffer
//! (`OpenFile::md`); `ui/viewer` draws it for the `Preview` and `Split` layouts.

pub mod annotation;
pub mod blockedit;
pub mod blocks;
pub mod events;
pub mod fences;
pub mod highlight;
pub mod inline;
pub mod minimap;
pub mod outline;
pub mod prose;
pub mod search;
pub mod structure;
pub mod sync;
pub mod view;

use crate::state::editor::MdReading;
use crate::theme::{MdDensity, MdMinimapSide, MdWidth};

/// What a markdown view is drawn with that the window, not the document, decides: the width
/// preset and density from `UiSettings`, the per-document reading options from the tab, and the
/// minimap settings. Pushed into the view by `AppState` whenever one of them changes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MdViewConfig {
    pub md_width: MdWidth,
    pub md_density: MdDensity,
    pub reading: MdReading,
    pub minimap: bool,
    pub minimap_side: MdMinimapSide,
}
