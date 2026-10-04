//! The markdown engine: a source-ranged block table. Pure Rust — no GPUI type appears here.
//!
//! The invariant the viewer rests on is stated in `markdown-improvement-proposal` §3:
//!
//! > Every row knows its source byte range, and the list knows every row's measured bounds.
//!
//! This crate owns the first half. [`parse`] turns a source string into a [`Document`] whose
//! [`Block`]s are the root-level elements in document order, each carrying the exact byte range it
//! came from. Given that, position-to-source and source-to-position are both a binary search, and
//! the minimap, the split-scroll sync and the anchors all stop guessing from byte ratios.
//!
//! Nothing here re-reads the source after parsing: a renderer gets a flat inline span list per
//! block ([`Inline`]) and can build styled runs from it without a second parse. That is the point —
//! the tree parses markdown four times today (§10), and the block table is what collapses that.
//!
//! Prior art: Zed's `markdown` crate is GPL-3.0 and none of it is copied here. The flat
//! source-ranged event stream is an idea, and this is built from `pulldown-cmark`'s own
//! `into_offset_iter()`.

use std::collections::HashMap;
use std::ops::Range;

use serde::{Deserialize, Serialize};

mod autolink;
mod build;
mod find;

#[cfg(test)]
mod tests;

pub use autolink::{find_autolinks, linkify_spans};
pub use find::{FindOptions, Match};

/// A block, with the source byte range it came from.
///
/// `ix` is the block's position among its *siblings* — at the root that is the list row index the
/// viewer addresses; inside a list item or a block quote it restarts at zero.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub ix: usize,
    pub kind: BlockKind,
    pub range: Range<usize>,
    /// A cheap stable hash of `source[range]`. Two parses of the same text agree; this is what
    /// [`Document::reparse`] diffs on, and what §4 row 13 wants for diff and streaming.
    pub hash: u64,
    /// The block's own inline content, flattened. Empty for container kinds (list, quote, table,
    /// footnote definition), which carry their inlines on the nested structure instead.
    pub spans: Vec<(Range<usize>, Inline)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BlockKind {
    /// A metadata block at the very top of the file. Front matter is *not* prose and must never be
    /// laid out as a paragraph.
    FrontMatter {
        style: FrontMatterStyle,
        body: String,
    },
    Heading {
        level: u8,
        text: String,
        slug: String,
    },
    Paragraph,
    /// A paragraph whose whole content is one image. Worth its own kind because it is a breakout
    /// element (§13.2) and gets a row rule, not a text rule.
    Image {
        dest: String,
        title: String,
        alt: String,
    },
    List {
        ordered: bool,
        /// The first number of an ordered list, as written.
        start: Option<u64>,
        /// Tight lists set no space between items. Derived from whether the parser wrapped item
        /// content in paragraphs, which is exactly CommonMark's own definition.
        tight: bool,
        items: Vec<ListItem>,
    },
    /// A code block. `body` is byte-identical to the code the source held, fences removed.
    Code {
        language: Option<String>,
        body: String,
        fenced: bool,
    },
    Quote {
        admonition: Option<Admonition>,
        blocks: Vec<Block>,
    },
    Table {
        alignments: Vec<ColumnAlign>,
        head: Vec<Cell>,
        rows: Vec<Vec<Cell>>,
    },
    ThematicBreak,
    Html {
        body: String,
    },
    FootnoteDefinition {
        label: String,
        blocks: Vec<Block>,
    },
    /// Anything the parser produced that this crate does not model yet. A renderer falls back to
    /// painting `source[range]` as prose.
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrontMatterStyle {
    /// `---` delimited.
    Yaml,
    /// `+++` delimited.
    Pluses,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Admonition {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

/// `Default` is the column with no alignment marker; it is not `Option` so the vector stays flat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColumnAlign {
    Default,
    Left,
    Center,
    Right,
}

/// One item of a list. `checked` is `Some` only for task-list items.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListItem {
    pub range: Range<usize>,
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

/// One table cell: its source range and its inlines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cell {
    pub range: Range<usize>,
    pub spans: Vec<(Range<usize>, Inline)>,
}

/// One entry of a block's flat inline stream.
///
/// The stream is the shape §3/§10 describe: a `Vec<(Range<usize>, Inline)>` where nesting is
/// expressed by balanced [`Inline::Enter`] / [`Inline::Exit`] pairs rather than by a tree. A
/// renderer walks it once with a mark stack and emits styled runs; every entry carries the source
/// bytes it came from, so a run maps back to an offset without re-parsing.
///
/// An `Enter`/`Exit` pair both carry the *whole* element's range, so a renderer that only wants the
/// extent of a link never has to pair them up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Inline {
    /// Literal text, already unescaped — it is not always equal to `source[range]`.
    Text(String),
    /// Inline code, fence backticks removed.
    Code(String),
    /// Raw inline HTML, e.g. `<br>`.
    Html(String),
    /// `[^label]`.
    FootnoteReference(String),
    /// The `[ ]` / `[x]` of a task-list item. Also recorded on [`ListItem::checked`].
    TaskMarker(bool),
    /// A newline inside a block that reflows away.
    SoftBreak,
    /// A newline the author forced.
    HardBreak,
    Enter(Mark),
    Exit(Mark),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Mark {
    Emphasis,
    Strong,
    Strikethrough,
    /// `title` is empty when the source gave none.
    Link {
        dest: String,
        title: String,
    },
    /// The alt text is the `Text` entries between `Enter` and `Exit`.
    Image {
        dest: String,
        title: String,
    },
}

/// A heading, richer than [`Document::outline`]'s tuple: enough to resolve an anchor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heading {
    /// Index into [`Document::blocks`] — the list row to reveal.
    pub block_ix: usize,
    pub level: u8,
    pub text: String,
    pub slug: String,
    pub range: Range<usize>,
}

/// What changed between two parses, in root block indices.
///
/// `blocks[first_changed .. first_changed + old_len]` became
/// `blocks[first_changed .. first_changed + new_len]`. That is the argument shape GPUI's
/// `ListState::splice(old_range, count)` wants, which is the whole reason this type exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockDelta {
    pub first_changed: usize,
    pub old_len: usize,
    pub new_len: usize,
}

impl BlockDelta {
    /// Nothing moved — the caller can skip the splice entirely.
    pub fn is_empty(&self) -> bool {
        self.old_len == 0 && self.new_len == 0
    }
}

/// A parsed document: the source plus its block table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Document {
    pub source: String,
    pub blocks: Vec<Block>,
}

/// Parse a document into its block table.
pub fn parse(source: &str) -> Document {
    let mut blocks = build::blocks(source);
    assign_slugs(&mut blocks, &mut HashMap::new());
    // `pulldown-cmark` leaves bare URLs as plain text and has no flag for it, so GFM
    // linkification is a post-pass over the spans; see [`autolink`].
    autolink::linkify_blocks(&mut blocks);
    Document {
        source: source.to_string(),
        blocks,
    }
}

impl Document {
    /// The block containing a source byte offset, or `None` in the whitespace between blocks.
    pub fn block_at_offset(&self, offset: usize) -> Option<usize> {
        let ix = self.block_at_or_before_offset(offset)?;
        (offset < self.blocks[ix].range.end).then_some(ix)
    }

    /// The block containing the offset, or the last one that starts before it.
    ///
    /// Scroll sync and anchor resolution land on blank lines constantly, and snapping backwards is
    /// always the answer there — a blank line belongs to the block it follows.
    pub fn block_at_or_before_offset(&self, offset: usize) -> Option<usize> {
        // `partition_point` is the binary search the invariant buys us: the block table is sorted
        // by `range.start` and the ranges do not overlap at the root.
        match self.blocks.partition_point(|b| b.range.start <= offset) {
            0 => None,
            n => Some(n - 1),
        }
    }

    /// A source offset as a block index plus an offset *within* that block's source.
    pub fn offset_to_block_and_local_offset(&self, offset: usize) -> Option<(usize, usize)> {
        let ix = self.block_at_offset(offset)?;
        Some((ix, offset - self.blocks[ix].range.start))
    }

    pub fn range_of_block(&self, ix: usize) -> Option<Range<usize>> {
        self.blocks.get(ix).map(|b| b.range.clone())
    }

    /// The source text of a block — what a row renderer is handed.
    pub fn block_source(&self, ix: usize) -> Option<&str> {
        self.blocks.get(ix).map(|b| &self.source[b.range.clone()])
    }

    /// Headings, in document order: `(block index, level, text)`.
    pub fn outline(&self) -> Vec<(usize, u8, String)> {
        self.blocks
            .iter()
            .filter_map(|b| match &b.kind {
                BlockKind::Heading { level, text, .. } => Some((b.ix, *level, text.clone())),
                _ => None,
            })
            .collect()
    }

    /// Headings with their slug and source range — enough to resolve `G138`/`G296` anchors.
    ///
    /// Only root-level headings appear: a heading nested in a quote or a list item is not a section
    /// and does not belong in a table of contents.
    pub fn headings(&self) -> Vec<Heading> {
        self.blocks
            .iter()
            .filter_map(|b| match &b.kind {
                BlockKind::Heading { level, text, slug } => Some(Heading {
                    block_ix: b.ix,
                    level: *level,
                    text: text.clone(),
                    slug: slug.clone(),
                    range: b.range.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    /// The root block index a `#slug` anchor points at.
    pub fn block_for_slug(&self, slug: &str) -> Option<usize> {
        self.blocks.iter().position(|b| match &b.kind {
            BlockKind::Heading { slug: s, .. } => s == slug,
            _ => false,
        })
    }

    /// Replace the source and report which root blocks changed.
    ///
    /// Deliberately a full reparse followed by a hash diff from both ends. It is the dumb
    /// implementation and it is the right one: parsing 150KB is well under the frame budget, and
    /// what the caller actually needs is the *splice range*, not a clever parser. An edit inside
    /// one paragraph yields a delta of one block whatever the file's size.
    pub fn reparse(&mut self, new_source: &str) -> BlockDelta {
        let new_doc = parse(new_source);
        let (old, new) = (&self.blocks, &new_doc.blocks);

        let mut first = 0;
        while first < old.len() && first < new.len() && old[first].hash == new[first].hash {
            first += 1;
        }
        // Then from the bottom, never crossing the prefix — an insertion must not be counted as
        // unchanged at both ends.
        let mut tail = 0;
        while tail < old.len() - first
            && tail < new.len() - first
            && old[old.len() - 1 - tail].hash == new[new.len() - 1 - tail].hash
        {
            tail += 1;
        }

        let delta = BlockDelta {
            first_changed: first,
            old_len: old.len() - first - tail,
            new_len: new.len() - first - tail,
        };
        *self = new_doc;
        delta
    }
}

/// FNV-1a over the block's source bytes.
///
/// Hand-rolled on purpose: the hash only has to be stable within a process run and cheap enough to
/// compute for every block on every reparse, which rules a cryptographic digest out and rules a new
/// dependency out with it.
pub fn content_hash(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// GitHub's heading anchor rule: lowercase, drop punctuation, spaces become hyphens.
///
/// Deliberately does not de-duplicate — that needs the whole document, and [`slug_deduped`] is the
/// one that walks it.
pub fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if c == ' ' || c == '-' || c == '_' {
            out.push('-');
        }
        // Everything else — punctuation, emoji, backticks — is dropped, as GitHub does.
    }
    out
}

/// [`slugify`], then GitHub's collision suffix: the second `## Notes` is `notes-1`.
///
/// `seen` counts occurrences per base slug rather than holding every slug emitted — a 350KB
/// document has hundreds of headings and a linear scan per heading is the one quadratic thing that
/// would show up here.
fn slug_deduped(text: &str, seen: &mut HashMap<String, usize>) -> String {
    let base = slugify(text);
    let n = seen.entry(base.clone()).or_insert(0);
    *n += 1;
    match *n {
        1 => base,
        n => format!("{base}-{}", n - 1),
    }
}

/// Fill in every heading's slug, in document order, so the counters match what a reader sees.
fn assign_slugs(blocks: &mut [Block], seen: &mut HashMap<String, usize>) {
    for block in blocks {
        match &mut block.kind {
            BlockKind::Heading { text, slug, .. } => *slug = slug_deduped(text, seen),
            BlockKind::Quote { blocks, .. } | BlockKind::FootnoteDefinition { blocks, .. } => {
                assign_slugs(blocks, seen)
            }
            BlockKind::List { items, .. } => {
                for item in items {
                    assign_slugs(&mut item.blocks, seen);
                }
            }
            _ => {}
        }
    }
}
