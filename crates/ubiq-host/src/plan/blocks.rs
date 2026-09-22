//! A plan's blocks, and the matching that carries their ids across a save.
//!
//! This is the weak point of the annotation design and the spec says so: an annotation anchors to
//! a [`BlockId`], so **what an id means is decided entirely here** — by walking the newly saved
//! document and pairing its blocks against the previous save's. Character offsets are refused
//! (`_docs/wip/planning-system.md`, decision 4): any insertion above a range moves everything
//! below it, silently.
//!
//! The rules, in the order they are applied:
//!
//! 1. **An identical block keeps its id.** Same kind, same text. A pure insertion, a pure deletion
//!    and a reorder are all this pass and nothing else.
//! 2. **An edited block keeps its id**, when it is still recognisably the same block: same kind,
//!    and a word-overlap similarity at or above [`SIMILAR`]. This is what "an edit inside a block
//!    keeps the anchor" means mechanically.
//! 3. **An unmatched new block is new** and is minted a fresh id.
//! 4. **An unmatched previous block vanished**, and its id is reported in [`Matching::vanished`]
//!    so its annotations can be flagged orphaned. Never dropped, never silently re-pointed at
//!    whatever took its place — a wrong anchor is worse than an admitted missing one.
//!
//! **The ambiguous case — two blocks with identical text — is settled by position.** Among the
//! previous blocks that would match equally well, the one nearest in document order wins, and a
//! tie between two equally distant candidates goes to the earlier one. Nothing better is available
//! without content that distinguishes them, and position is at least stable: re-saving an
//! unchanged document maps every duplicate onto itself, which is the case that actually happens.
//!
//! The parser is the interface's own — the `markdown` crate at `ParseOptions::gfm()`, which is
//! what `crates/ubiq/src/ui/viewer/markdown.rs` and gpui-component's text view parse with — so a
//! block indexed here is a block the preview draws, and the user annotates what they see.

use markdown::mdast::Node;
use ubiq_proto::ids::BlockId;
use ubiq_proto::plan::PlanBlock;

/// How much of two blocks' words must be shared for one to be the other, edited. Deliberately
/// generous: a block whose wording was half rewritten is still the block the annotation was about,
/// and the alternative — orphaning it — is the loud failure, not the quiet one.
const SIMILAR: f64 = 0.5;

/// One block of a parsed plan, before it has an id: what the document says now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// The mdast node kind, with a heading's depth folded in. Two blocks of different kinds never
    /// match, so a paragraph cannot inherit a heading's id by resembling it.
    pub kind: String,
    /// The block's own source, trimmed: the slice of the document the parser gave it. Markdown
    /// rather than rendered text — a fence's ``` and a heading's `#` are part of what it is, while
    /// a list item's bullet is not, because the parser hands back the item's paragraph.
    pub text: String,
}

/// One block of a plan as the last save indexed it — [`ubiq_proto::plan::PlanBlock`], because the
/// index the sidecar stores and the index a window is handed are the same list. The sidecar is the
/// only place a [`BlockId`] is written down: never in the markdown, which stays plain markdown.
pub type IndexedBlock = PlanBlock;

/// What one save's matching decided.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Matching {
    /// The new document's blocks, in document order, each carrying the id it kept or the one it
    /// was minted. This is the next save's `previous`.
    pub blocks: Vec<IndexedBlock>,
    /// Previous ids nothing in the new document matched. Every annotation naming one of these is
    /// orphaned — flagged, kept, and shown as such.
    pub vanished: Vec<BlockId>,
}

/// The blocks of a plan, in document order.
///
/// Container nodes are walked through rather than emitted, so a list annotates per item and a
/// quote per paragraph; a table is one block, because its rows are not what anyone selects. A
/// document that will not parse has no blocks, which makes every previous block vanish rather than
/// silently keeping an index that no longer describes the file.
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
fn kind_of(node: &Node) -> Option<String> {
    Some(match node {
        Node::Paragraph(_) => "paragraph".to_string(),
        // A heading's depth is part of its kind: promoting `###` to `##` is a restructure, and the
        // similarity pass must not quietly carry an annotation across it.
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

/// Carry the previous save's ids onto the new document's blocks. Pure: the only thing it reaches
/// for is a fresh [`BlockId`] for a block nothing matched.
pub fn match_blocks(previous: &[IndexedBlock], current: &[Block]) -> Matching {
    // `matched[i]` is the previous block the current block at `i` took its id from.
    let mut matched: Vec<Option<usize>> = vec![None; current.len()];
    let mut taken = vec![false; previous.len()];

    // Pass 1 — identical text. Position settles a tie, which is the whole answer to two identical
    // blocks: nearest in document order, then earliest.
    for (i, block) in current.iter().enumerate() {
        let best = previous
            .iter()
            .enumerate()
            .filter(|(j, candidate)| {
                !taken[*j] && candidate.kind == block.kind && candidate.text == block.text
            })
            .min_by_key(|(j, _)| (j.abs_diff(i), *j))
            .map(|(j, _)| j);
        if let Some(j) = best {
            matched[i] = Some(j);
            taken[j] = true;
        }
    }

    // Pass 2 — an edit inside a block. Every surviving pair is scored, and the best pairs are
    // taken first so that a block's own edited self beats a vaguely similar neighbour, whatever
    // order the document happens to be in.
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (i, block) in current.iter().enumerate() {
        if matched[i].is_some() {
            continue;
        }
        for (j, candidate) in previous.iter().enumerate() {
            if taken[j] || candidate.kind != block.kind {
                continue;
            }
            let score = similarity(&candidate.text, &block.text);
            if score >= SIMILAR {
                pairs.push((score, i, j));
            }
        }
    }
    // Best score first; then the nearest in document order, then the earliest — so the result
    // does not depend on the order the pairs were generated in.
    pairs.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| a.1.abs_diff(a.2).cmp(&b.1.abs_diff(b.2)))
            .then_with(|| (a.1, a.2).cmp(&(b.1, b.2)))
    });
    for (_, i, j) in pairs {
        if matched[i].is_none() && !taken[j] {
            matched[i] = Some(j);
            taken[j] = true;
        }
    }

    let blocks = current
        .iter()
        .enumerate()
        .map(|(i, block)| IndexedBlock {
            id: matched[i].map_or_else(BlockId::generate, |j| previous[j].id),
            kind: block.kind.clone(),
            text: block.text.clone(),
        })
        .collect();
    let vanished = previous
        .iter()
        .enumerate()
        .filter(|(j, _)| !taken[*j])
        .map(|(_, block)| block.id)
        .collect();

    Matching { blocks, vanished }
}

/// How much two blocks have in common, as a Sørensen–Dice coefficient over their words: twice the
/// shared words over the two word counts, `1.0` for the same words and `0.0` for nothing in
/// common. Words rather than characters, because a rewritten sentence shares its characters with
/// almost anything and a paragraph must not drift onto its neighbour's id.
///
/// The words are [`words`]', which is what makes this usable on markdown rather than on prose: a
/// heading's `#` and a list item's `-` are punctuation both sides always share, and counting them
/// would make `# Old` and `# New` half the same block.
fn similarity(left: &str, right: &str) -> f64 {
    let left = words(left);
    let mut right: Vec<Option<String>> = words(right).into_iter().map(Some).collect();
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let total = left.len() + right.len();
    let mut shared = 0usize;
    for word in &left {
        if let Some(slot) = right
            .iter_mut()
            .find(|slot| slot.as_deref() == Some(word.as_str()))
        {
            *slot = None;
            shared += 1;
        }
    }
    2.0 * shared as f64 / total as f64
}

/// A block's words, for comparison only: lowercased, stripped of the punctuation markdown and
/// prose hang off them, and emptied of the tokens that were nothing but punctuation. `Step two.`
/// and `Step two, revised.` have to read as the same block edited, and `- one` and `1. one` as the
/// same item renumbered.
fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn indexed(blocks: Vec<Block>) -> Vec<IndexedBlock> {
        match_blocks(&[], &blocks).blocks
    }

    fn ids(matching: &Matching) -> Vec<BlockId> {
        matching.blocks.iter().map(|block| block.id).collect()
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
        assert_eq!(blocks[4].text, "```rust\nfn main() {}\n```");
    }

    #[test]
    fn an_empty_previous_version_mints_every_id() {
        let current = blocks("# Title\n\nA paragraph.\n");
        let matching = match_blocks(&[], &current);

        assert_eq!(matching.blocks.len(), 2);
        assert!(matching.vanished.is_empty());
        assert_ne!(matching.blocks[0].id, matching.blocks[1].id);
    }

    #[test]
    fn an_unchanged_document_keeps_every_id() {
        let source = "# Title\n\nA paragraph.\n\nAnother.\n";
        let previous = indexed(blocks(source));
        let matching = match_blocks(&previous, &blocks(source));

        assert_eq!(
            ids(&matching),
            previous.iter().map(|b| b.id).collect::<Vec<_>>()
        );
        assert!(matching.vanished.is_empty());
    }

    #[test]
    fn a_pure_insertion_keeps_every_old_id_and_mints_one() {
        let previous = indexed(blocks("# Title\n\nFirst.\n\nSecond.\n"));
        let matching = match_blocks(
            &previous,
            &blocks("# Title\n\nFirst.\n\nInserted.\n\nSecond.\n"),
        );

        assert!(matching.vanished.is_empty(), "nothing was removed");
        assert_eq!(matching.blocks.len(), 4);
        assert_eq!(matching.blocks[0].id, previous[0].id);
        assert_eq!(matching.blocks[1].id, previous[1].id);
        assert_eq!(
            matching.blocks[3].id, previous[2].id,
            "the block after the insertion kept its id — no offset moved",
        );
        assert!(
            !previous.iter().any(|b| b.id == matching.blocks[2].id),
            "the inserted block is new",
        );
    }

    #[test]
    fn a_pure_deletion_reports_exactly_the_block_that_went() {
        let previous = indexed(blocks("# Title\n\nFirst.\n\nSecond.\n"));
        let matching = match_blocks(&previous, &blocks("# Title\n\nSecond.\n"));

        assert_eq!(matching.vanished, vec![previous[1].id]);
        assert_eq!(ids(&matching), vec![previous[0].id, previous[2].id]);
    }

    #[test]
    fn a_reorder_moves_the_ids_with_the_blocks() {
        let previous = indexed(blocks("First.\n\nSecond.\n\nThird.\n"));
        let matching = match_blocks(&previous, &blocks("Third.\n\nFirst.\n\nSecond.\n"));

        assert!(matching.vanished.is_empty());
        assert_eq!(
            ids(&matching),
            vec![previous[2].id, previous[0].id, previous[1].id],
        );
    }

    #[test]
    fn an_edit_inside_a_block_keeps_the_anchor() {
        let previous = indexed(blocks(
            "## Design\n\nThe coordinator owns the panes and nothing else.\n",
        ));
        let matching = match_blocks(
            &previous,
            &blocks("## Design\n\nThe coordinator owns the panes and their lifecycle.\n"),
        );

        assert!(
            matching.vanished.is_empty(),
            "an edited block did not vanish",
        );
        assert_eq!(ids(&matching), vec![previous[0].id, previous[1].id]);
    }

    #[test]
    fn a_short_block_survives_a_word_being_added_to_it() {
        // The case punctuation would break: `two.` and `two,` are not the same token until the
        // words are normalised, and a two-word paragraph has no margin to lose.
        let previous = indexed(blocks("Step two.\n"));
        let matching = match_blocks(&previous, &blocks("Step two, revised.\n"));

        assert_eq!(ids(&matching), vec![previous[0].id]);
        assert!(matching.vanished.is_empty());
    }

    #[test]
    fn a_wholesale_rewrite_vanishes_everything() {
        let previous = indexed(blocks("# Old\n\nEntirely about one thing.\n"));
        let matching = match_blocks(
            &previous,
            &blocks("# New\n\nA completely different subject, unrelated words throughout.\n"),
        );

        assert_eq!(matching.vanished.len(), 2, "both old blocks are reported");
        assert!(
            matching
                .blocks
                .iter()
                .all(|block| !previous.iter().any(|old| old.id == block.id)),
            "every block is new",
        );
    }

    #[test]
    fn two_identical_blocks_are_settled_by_position() {
        let previous = indexed(blocks("Same.\n\nMiddle.\n\nSame.\n"));
        // The first `Same.` is still first and the second is still last: each maps onto itself.
        let matching = match_blocks(&previous, &blocks("Same.\n\nMiddle.\n\nSame.\n"));
        assert_eq!(
            ids(&matching),
            vec![previous[0].id, previous[1].id, previous[2].id],
        );

        // Drop the middle block and the two duplicates close up. Nearest-in-order wins, so the
        // surviving pair is the first and the second in the order they were, and nothing vanishes.
        let matching = match_blocks(&previous, &blocks("Same.\n\nSame.\n"));
        assert_eq!(ids(&matching), vec![previous[0].id, previous[2].id]);
        assert_eq!(matching.vanished, vec![previous[1].id]);

        // One of the two duplicates goes. The one that is left takes the nearer id — the first —
        // and the second is reported vanished rather than both being guessed at.
        let matching = match_blocks(&previous, &blocks("Same.\n\nMiddle.\n"));
        assert_eq!(ids(&matching), vec![previous[0].id, previous[1].id]);
        assert_eq!(matching.vanished, vec![previous[2].id]);
    }

    #[test]
    fn a_block_never_takes_a_different_kinds_id() {
        let previous = indexed(blocks("## The plan is to ship it\n"));
        let matching = match_blocks(&previous, &blocks("The plan is to ship it\n"));

        assert_eq!(
            matching.vanished,
            vec![previous[0].id],
            "a heading turned into a paragraph is a new block and an orphaned annotation",
        );
    }

    #[test]
    fn an_empty_document_vanishes_every_block() {
        let previous = indexed(blocks("# Title\n\nA paragraph.\n"));
        let matching = match_blocks(&previous, &blocks(""));

        assert!(matching.blocks.is_empty());
        assert_eq!(matching.vanished.len(), 2);
    }
}
