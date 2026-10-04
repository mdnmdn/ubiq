//! What the navigator navigates: a flat, ordered, depth-annotated list of places.
//!
//! The navigator — the left dock and the jump popover alike (`ui/mdview/outline.rs`, T-301) —
//! draws any [`Structure`] and knows nothing about markdown. The document's headings are one
//! structure ([`Outline`]); a file list, a symbol list or the annotation threads would be others,
//! each a new impl plus, if it lands somewhere new, a new [`Target`] variant.
//!
//! No GPUI type here: what activating a [`Target`] *does* is the interface's business
//! (the navigator's), so this module stays testable without a window.

use ubiq_md::Document;

/// Where activating an item takes the reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A root block index — a preview `list()` row, so `MdView::reveal_block` is the whole jump.
    Block(usize),
}

/// One place in a structure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub target: Target,
    /// Nesting depth, `0` at the top. The navigator indents by it.
    pub depth: u8,
    pub label: String,
}

/// A navigable structure: its items in display order, and which one the reader is inside.
pub trait Structure {
    /// What the structure is called — the popover's placeholder and the empty state say it.
    fn title(&self) -> &str;
    fn items(&self) -> &[Item];
    /// Index into [`Self::items`] of the item the reader is currently inside, if one is known.
    fn current(&self) -> Option<usize>;
}

/// The indices of the items whose label contains every whitespace-separated word of `query`,
/// case-insensitively, in display order. An empty query keeps everything.
pub fn filter(structure: &dyn Structure, query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    structure
        .items()
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            let label = item.label.to_lowercase();
            words.iter().all(|word| label.contains(word.as_str()))
        })
        .map(|(n, _)| n)
        .collect()
}

/// Which heading the preview is currently inside: the last heading at or before the first block
/// the viewport shows.
///
/// `heading_blocks` is `Document::outline()`'s block indices, ascending. Returns an index *into
/// that slice*, so the navigator can mark one of its own rows without a second lookup. Here rather
/// than in the minimap (T-301) so both read the one answer.
pub fn current_section(heading_blocks: &[usize], top_block: usize) -> Option<usize> {
    match heading_blocks.partition_point(|&ix| ix <= top_block) {
        0 => None,
        n => Some(n - 1),
    }
}

/// A markdown document's headings, from the block table — `Document::outline()`, unchanged.
pub struct Outline {
    items: Vec<Item>,
    current: Option<usize>,
}

impl Outline {
    /// `top_block` is the preview's first visible root block, or `None` when no block list is on
    /// screen — a highlight pinned to the first heading would be a lie about a document nobody is
    /// looking at through that list.
    pub fn new(doc: &Document, top_block: Option<usize>) -> Self {
        let headings = doc.outline();
        // The section containing the top row is the last heading at or before it: a row index
        // against row indices, with no pixel or byte arithmetic involved.
        let current = top_block.and_then(|top| {
            let blocks: Vec<usize> = headings.iter().map(|(ix, _, _)| *ix).collect();
            current_section(&blocks, top)
        });
        let items = headings
            .into_iter()
            .map(|(ix, level, text)| Item {
                target: Target::Block(ix),
                depth: level.saturating_sub(1),
                label: text,
            })
            .collect();
        Outline { items, current }
    }
}

impl Structure for Outline {
    fn title(&self) -> &str {
        "headings"
    }

    fn items(&self) -> &[Item] {
        &self.items
    }

    fn current(&self) -> Option<usize> {
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str =
        "# Intro\n\ntext\n\n## Getting started\n\nmore\n\n### Install the tool\n\n## Usage\n";

    #[test]
    fn outline_maps_headings_to_blocks_and_depths() {
        let doc = ubiq_md::parse(DOC);
        let outline = Outline::new(&doc, None);
        let shape: Vec<(u8, &str)> = outline
            .items()
            .iter()
            .map(|item| (item.depth, item.label.as_str()))
            .collect();
        assert_eq!(
            shape,
            [
                (0, "Intro"),
                (1, "Getting started"),
                (2, "Install the tool"),
                (1, "Usage")
            ]
        );
        assert_eq!(outline.items()[0].target, Target::Block(0));
        assert_eq!(outline.current(), None);
    }

    #[test]
    fn current_is_the_last_heading_at_or_before_the_top_row() {
        let doc = ubiq_md::parse(DOC);
        let Target::Block(install) = Outline::new(&doc, None).items()[2].target;
        assert_eq!(Outline::new(&doc, Some(install)).current(), Some(2));
        // The paragraph just before it still belongs to "Getting started".
        assert_eq!(Outline::new(&doc, Some(install - 1)).current(), Some(1));
        assert_eq!(Outline::new(&doc, Some(1)).current(), Some(0));
    }

    #[test]
    fn filter_matches_every_word_case_insensitively() {
        let doc = ubiq_md::parse(DOC);
        let outline = Outline::new(&doc, None);
        assert_eq!(filter(&outline, ""), [0, 1, 2, 3]);
        assert_eq!(filter(&outline, "  "), [0, 1, 2, 3]);
        assert_eq!(filter(&outline, "START"), [1]);
        assert_eq!(filter(&outline, "the install"), [2]);
        assert_eq!(filter(&outline, "s"), [1, 2, 3]);
        assert!(filter(&outline, "nothing").is_empty());
    }
}
