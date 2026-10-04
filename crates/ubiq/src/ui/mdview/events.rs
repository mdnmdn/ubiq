//! What an [`MdView`](super::view::MdView) tells whoever embeds it.
//!
//! The view is an `EventEmitter<MdViewEvent>`: every link click, selection, block edit and reparse
//! goes out as one of these, emitted from the code path that made it, so the host subscribes once
//! and never reaches into the view's fields. Block indices are as of the parse the event was
//! emitted against; one kept past the next [`MdViewEvent::DocumentChanged`] is a claim, not a fact.

use std::ops::Range;

use gpui::SharedString;
use ubiq_proto::plan::{AnnotationMark, HighlightColour};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MdViewEvent {
    /// A link in the rendered prose was clicked; the destination as written in the source.
    LinkClicked(SharedString),
    /// The selected root block changed.
    BlockSelected { block: usize },
    /// The block editor committed: `old_range` of the source (as of the parse it opened on) was
    /// replaced by `new_text`. `blocks` is the span of root blocks the editor stood in for. The
    /// reparse that follows emits [`Self::DocumentChanged`].
    BlockEdited {
        blocks: Range<usize>,
        old_range: Range<usize>,
        new_text: String,
    },
    /// The block editor on the span `blocks` was closed without writing.
    EditCancelled { blocks: Range<usize> },
    /// The block table was rebuilt from an edited source — by the source editor or a block commit.
    DocumentChanged { blocks: usize },
    /// The reader scrolled the list: `rows` are the root blocks now on screen.
    Scrolled { rows: Range<usize> },
    // ── Annotation intents (`annotation.rs`). The view mutates no decor of its own: the host
    // acts, and the decor it pushes back (`MdView::set_decor`) is what the margin then draws.
    /// `+` on `row`: open a new thread on it.
    AddThread { row: usize },
    /// The thread-count marker on `row`: show its threads.
    ThreadsClicked { row: usize },
    /// A pick from `row`'s `⚑` menu: toggle `mark` on its target thread (or start one with it).
    MarkRequested { row: usize, mark: AnnotationMark },
    /// `✓` (`resolved: true`) or `↺` (`false`) on `row`.
    ResolveRequested { row: usize, resolved: bool },
    /// A pick from `row`'s `●` menu; `None` clears the highlight.
    HighlightRequested {
        row: usize,
        colour: Option<HighlightColour>,
    },
}

/// `block 3` for a one-block span, `blocks 3-4` (inclusive) for a wider one.
pub fn span_label(blocks: &Range<usize>) -> String {
    if blocks.len() <= 1 {
        format!("block {}", blocks.start)
    } else {
        format!("blocks {}-{}", blocks.start, blocks.end - 1)
    }
}
