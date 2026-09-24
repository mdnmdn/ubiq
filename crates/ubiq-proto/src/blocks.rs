//! Splitting a markdown document into the blocks an annotation can anchor to.
//!
//! **This is shared logic, not a message.** It lives here because both halves have to split a
//! document *the same way* and neither may depend on the other: the host indexes a saved plan into
//! [`crate::plan::PlanBlock`]s (`crates/ubiq-host/src/plan/blocks.rs`, which matches ids across a
//! save on top of this walk), and the window runs the same walk to patch its own block cache
//! optimistically after a section edit, ahead of the host's re-index. Two copies of the rules is
//! exactly the drift that would make the cache disagree with the index, so there is one.
//!
//! The parser is the interface's own — the `markdown` crate at [`markdown::ParseOptions::gfm`],
//! which is what `crates/ubiq/src/ui/viewer/markdown.rs` and gpui-component's text view parse with
//! — so a block split out here is a block the preview draws, and the user annotates what they see.
//!
//! The rules:
//!
//! - **Container nodes are walked through rather than emitted**, so a list annotates per item and a
//!   quote per paragraph. A table is *not* a container: it is one block, because its rows are not
//!   what anyone selects.
//! - **A node that is not block-level is not a block.** Phrasing inside a paragraph never becomes
//!   one of its own.
//! - **A block's text is its own source, trimmed** — markdown rather than rendered text, so a
//!   fence's ``` and a heading's `#` are part of what it is, while a list item's bullet and a
//!   quote's `>` are not, because what the parser hands back is the item's or the quote's own
//!   paragraph and its position starts after the marker.
//! - **A block that trims to nothing is dropped**, and **a document that will not parse has no
//!   blocks** — which, host-side, makes every previous block vanish rather than silently keeping an
//!   index that no longer describes the file.

use markdown::mdast::Node;

/// One block of a document, before it has an id: what the document says now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// The mdast node kind, with a heading's depth folded in. Two blocks of different kinds never
    /// match, so a paragraph cannot inherit a heading's id by resembling it.
    pub kind: String,
    /// The block's own source, trimmed.
    pub text: String,
}

impl Block {
    /// The block as a `(kind, text)` pair, for a caller that wants the two strings and not the
    /// struct.
    pub fn into_pair(self) -> (String, String) {
        (self.kind, self.text)
    }
}

/// The blocks of a document, in document order.
pub fn blocks(source: &str) -> Vec<Block> {
    let Ok(ast) = markdown::to_mdast(source, &markdown::ParseOptions::gfm()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    walk(&ast, source, &mut out);
    out
}

fn walk(node: &Node, source: &str, out: &mut Vec<Block>) {
    if is_container(node) {
        if let Some(children) = node.children() {
            for child in children {
                walk(child, source, out);
            }
        }
        return;
    }
    let (Some(kind), Some(position)) = (kind_of(node), node.position()) else {
        return;
    };
    let Some(text) = source.get(position.start.offset..position.end.offset) else {
        return;
    };
    let text = text.trim();
    if !text.is_empty() {
        out.push(Block {
            kind,
            text: text.to_string(),
        });
    }
}

/// A node that holds blocks rather than being one.
fn is_container(node: &Node) -> bool {
    matches!(
        node,
        Node::Root(_)
            | Node::Blockquote(_)
            | Node::List(_)
            | Node::ListItem(_)
            | Node::FootnoteDefinition(_)
    )
}

/// The kind two blocks have to share to be the same block. `None` for a node that is not
/// block-level — phrasing inside a paragraph is never a block of its own.
///
/// `math` and `frontmatter` are unreachable at [`markdown::ParseOptions::gfm`], which enables
/// neither construct; they are kept so that turning either on is a one-line change here rather
/// than a silently dropped block.
fn kind_of(node: &Node) -> Option<String> {
    Some(match node {
        Node::Paragraph(_) => "paragraph".to_string(),
        // A heading's depth is part of its kind: promoting `###` to `##` is a restructure, and the
        // host's similarity pass must not quietly carry an annotation across it.
        Node::Heading(heading) => format!("heading:{}", heading.depth),
        Node::Code(_) => "code".to_string(),
        Node::Math(_) => "math".to_string(),
        Node::Table(_) => "table".to_string(),
        Node::ThematicBreak(_) => "break".to_string(),
        Node::Html(_) => "html".to_string(),
        Node::Definition(_) => "definition".to_string(),
        Node::Toml(_) | Node::Yaml(_) => "frontmatter".to_string(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(source: &str) -> Vec<(String, String)> {
        blocks(source).into_iter().map(Block::into_pair).collect()
    }

    fn kinds(source: &str) -> Vec<String> {
        blocks(source).into_iter().map(|block| block.kind).collect()
    }

    #[test]
    fn a_document_splits_into_the_blocks_the_preview_draws() {
        let source = "# Title\n\nA paragraph.\n\n- one\n- two\n\n```rust\nfn main() {}\n```\n";
        let blocks = blocks(source);
        let kinds: Vec<&str> = blocks.iter().map(|b| b.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["heading:1", "paragraph", "paragraph", "paragraph", "code"],
            "a list is walked through to its items, which are paragraphs",
        );
        assert_eq!(blocks[0].text, "# Title");
        assert_eq!(
            blocks[4].text, "```rust\nfn main() {}\n```",
            "a fence's own delimiters are part of the block",
        );
    }

    #[test]
    fn an_empty_document_has_no_blocks() {
        assert!(blocks("").is_empty());
        assert!(blocks("   \n\n\t\n").is_empty(), "whitespace is not a block");
    }

    #[test]
    fn a_nested_list_yields_one_block_per_leaf_paragraph() {
        // Both containers — the outer list and the inner one — are walked through, so the item
        // that holds a sub-list contributes only its own paragraph, not the sub-list's text too.
        assert_eq!(
            split("- one\n  - nested\n- two\n"),
            vec![
                ("paragraph".to_string(), "one".to_string()),
                ("paragraph".to_string(), "nested".to_string()),
                ("paragraph".to_string(), "two".to_string()),
            ],
        );
    }

    #[test]
    fn a_quote_annotates_per_paragraph_and_drops_its_markers() {
        // The quote is a container, so what comes out is its paragraphs — and a paragraph's own
        // position starts *after* the `> `, so the marker is not in the text. Same rule as a list
        // item's bullet, and unlike a fence's ``` or a heading's `#`, which are the node's own.
        assert_eq!(
            split("> first\n>\n> second\n"),
            vec![
                ("paragraph".to_string(), "first".to_string()),
                ("paragraph".to_string(), "second".to_string()),
            ],
        );
    }

    #[test]
    fn a_table_is_one_block_and_not_walked_into() {
        let source = "| a | b |\n| - | - |\n| 1 | 2 |\n";
        let blocks = blocks(source);
        assert_eq!(blocks.len(), 1, "rows are not what anyone selects");
        assert_eq!(blocks[0].kind, "table");
        assert_eq!(blocks[0].text, source.trim());
    }

    #[test]
    fn a_thematic_break_an_html_block_and_a_definition_are_each_their_own_block() {
        assert_eq!(
            kinds("---\n\n<div>raw</div>\n\n[ref]: https://example.com\n"),
            vec!["break", "html", "definition"],
        );
    }

    #[test]
    fn a_headings_depth_is_part_of_its_kind() {
        assert_eq!(
            kinds("# one\n\n## two\n\n###### six\n"),
            vec!["heading:1", "heading:2", "heading:6"],
        );
    }

    #[test]
    fn frontmatter_is_not_a_construct_at_gfm_options() {
        // Pinned rather than assumed: `kind_of` has a `frontmatter` arm and it is dead, because
        // `ParseOptions::gfm()` does not enable the construct. A `---` fence becomes a thematic
        // break, the `title: x` line a setext heading closed by the second `---`, and the body an
        // ordinary paragraph. Wrong-looking, and deliberately recorded: it is what both halves do,
        // which is what matters, and turning frontmatter on is a `ParseOptions` change that would
        // have to be made in the interface's renderer at the same time.
        assert_eq!(
            kinds("---\ntitle: x\n---\n\nBody.\n"),
            vec!["break", "heading:2", "paragraph"],
        );
    }

    #[test]
    fn phrasing_inside_a_paragraph_is_never_its_own_block() {
        assert_eq!(
            split("Some **bold** and `code` and [a link](x).\n"),
            vec![(
                "paragraph".to_string(),
                "Some **bold** and `code` and [a link](x).".to_string(),
            )],
        );
    }

    #[test]
    fn a_footnote_definition_is_walked_through_to_its_paragraph() {
        assert_eq!(
            kinds("Text[^a]\n\n[^a]: The note.\n"),
            vec!["paragraph", "paragraph"],
        );
    }
}
