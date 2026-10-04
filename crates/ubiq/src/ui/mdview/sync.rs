//! Linked scrolling between the source and the preview, mapped through **block indices**.
//!
//! **This module names no GPUI type.** Everything here is a `&Document`, a `usize` and an `f32`,
//! which is what makes the mapping testable at all — the two halves of the split are a
//! `gpui::list()` and a text editor, and neither can be instantiated without a window.
//!
//! **Why indices and not fractions.** The base's viewer links its two panes by comparing each
//! side's scroll fraction every frame and inferring from the difference which one the user moved.
//! Two panes whose content has different total heights never agree on a fraction, so the inferred
//! driver flips, each side corrects the other, and the pair drifts. A block index is a fact both
//! halves already hold: the preview's row `n` *is* `doc.blocks[n]`, and that block knows its exact
//! source byte range. The mapping is therefore exact at every block boundary, and the only
//! approximation left is *inside* one block — which is what [`BlockPoint::fraction`] carries.
//!
//! The two directions are inverses:
//!
//! - **preview → source** is [`block_and_fraction_to_offset`]: a row index plus how far down that
//!   row the viewport sits, out to a source byte offset.
//! - **source → preview** is [`offset_to_block_and_fraction`]: a source byte offset in, a row
//!   index plus how far down that row to put the viewport.
//!
//! [`ScrollLink`] is the third piece: the flag that stops the write from coming back.
//!
//! **Reparse lives here too, for the same reason.** An edit in the source view produces a
//! [`ubiq_md::BlockDelta`], and the two things the interface has to do with it — splice the row
//! range, and carry the viewport's anchor across the splice — are index arithmetic and nothing
//! else. [`splice_range`] and [`carry_anchor`] are that arithmetic, testable without a window;
//! the `MdView` entity's reparse (T-301) is the ten lines that call them.

use std::cell::Cell;
use std::ops::Range;

use ubiq_md::{BlockDelta, Document};

/// A position in a document: which root block, and how far through it.
///
/// The pair is the whole contract between the two halves. A block index alone would snap — a
/// 200-line code fence is one row and one block, so a viewport anywhere inside it would map to the
/// fence's first byte and the other side would sit still until the fence went by. The fraction is
/// what makes that fence scroll.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockPoint {
    /// An index into `Document::blocks`, which is also the preview's `list()` row index.
    pub block: usize,
    /// Where in the block the viewport's top edge sits: `0.0` at its first byte, `1.0` at its
    /// last. Always in `0.0..=1.0` — [`BlockPoint::new`] is the only constructor that matters and
    /// it clamps.
    pub fraction: f32,
}

impl BlockPoint {
    /// A point, with the fraction clamped into range.
    ///
    /// Both callers hand in a ratio they measured, and both can exceed the range legitimately:
    /// the list reports an `offset_in_item` measured against a row height that has since changed,
    /// and a source offset can land in the gap *after* a block's last byte (see
    /// [`offset_to_block_and_fraction`]). Clamping here is what keeps every other function in this
    /// module total.
    pub fn new(block: usize, fraction: f32) -> Self {
        BlockPoint {
            block,
            fraction: if fraction.is_finite() {
                fraction.clamp(0.0, 1.0)
            } else {
                0.0
            },
        }
    }
}

/// **Preview → source.** Where in the source the preview's viewport top is pointing.
///
/// The block's source range is interpolated linearly by byte. That is not the same as
/// interpolating by line, and by byte is the right one: it is the only measure both halves can
/// compute without laying the other one out, and inside a single block the two agree closely
/// enough that the seam is invisible.
///
/// The answer is always a char boundary, walked backwards — `Document::source` is a `String` and
/// the caller slices it.
pub fn block_and_fraction_to_offset(doc: &Document, at: BlockPoint) -> Option<usize> {
    let range = doc.range_of_block(at.block)?;
    let span = range.end.saturating_sub(range.start);
    let into = (span as f32 * at.fraction.clamp(0.0, 1.0)).round() as usize;

    let mut offset = range.start.saturating_add(into).min(range.end);
    while offset > range.start && !doc.source.is_char_boundary(offset) {
        offset -= 1;
    }
    Some(offset)
}

/// **Source → preview.** Which row the preview should show, and how far into it.
///
/// **`block_at_or_before_offset`, never `block_at_offset`.** The offset arrives from the source
/// editor's top visible line, and a scrolling editor lands between blocks constantly: blank lines,
/// and link reference definitions, which CommonMark consumes into the link map and which therefore
/// produce *no block at all*. `block_at_offset` answers `None` for every one of those, and a
/// `None` here is a frame where the preview does not move. Snapping backwards is also the right
/// answer rather than merely the available one — a blank line belongs to the block above it.
///
/// An offset past a block's last byte clamps to `fraction == 1.0`, so the preview sits at the foot
/// of that row while the source crosses the gap. `None` only for an offset before the first block,
/// which is the empty document.
pub fn offset_to_block_and_fraction(doc: &Document, offset: usize) -> Option<BlockPoint> {
    let block = doc.block_at_or_before_offset(offset)?;
    let range = doc.range_of_block(block)?;
    let span = range.end.saturating_sub(range.start);

    // A zero-length block (a thematic break the parser gave an empty range) has no inside to be
    // partway through.
    let fraction = if span == 0 {
        0.0
    } else {
        offset.saturating_sub(range.start) as f32 / span as f32
    };
    Some(BlockPoint::new(block, fraction))
}

/// A pixel offset inside a measured row, as a fraction of that row.
///
/// The list reports its position as a row index plus pixels into that row; this is the half of the
/// conversion that needs the row's *measured* height, which only the laid-out list knows.
pub fn fraction_in_row(offset_in_row: f32, row_height: f32) -> f32 {
    if !row_height.is_finite() || row_height <= 0.0 || !offset_in_row.is_finite() {
        // An unmeasured row, or a row of no height: the top of it is the only position it has.
        return 0.0;
    }
    (offset_in_row / row_height).clamp(0.0, 1.0)
}

/// The inverse of [`fraction_in_row`]: how many pixels into a measured row a fraction sits.
///
/// A row taller than the viewport is the interesting case and needs no special handling — the
/// fraction is of the *row*, not of the viewport, so a 4000-pixel row in a 800-pixel viewport
/// still resolves every fraction to a distinct pixel.
pub fn pixels_in_row(fraction: f32, row_height: f32) -> f32 {
    if !row_height.is_finite() || row_height <= 0.0 {
        return 0.0;
    }
    (fraction.clamp(0.0, 1.0) * row_height).max(0.0)
}

/// The zero-based line a byte offset falls on. The source editor scrolls in lines, so this is the
/// last step of preview → source.
pub fn line_of_offset(source: &str, offset: usize) -> usize {
    let end = offset.min(source.len());
    source.as_bytes()[..end]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
}

/// The byte offset of a zero-based line's first byte — the first step of source → preview.
///
/// A line past the end answers the end, so the preview parks on the last block rather than
/// refusing to move.
pub fn offset_of_line(source: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    let mut seen = 0;
    for (ix, byte) in source.as_bytes().iter().enumerate() {
        if *byte == b'\n' {
            seen += 1;
            if seen == line {
                return ix + 1;
            }
        }
    }
    source.len()
}

// ── Reparse: the delta as list arithmetic ───────────────────────────

/// A [`BlockDelta`] as the two arguments `ListState::splice(old_range, count)` takes.
///
/// One row per root block is the identity the whole module rests on, so the delta's block indices
/// *are* row indices and this is a rename rather than a conversion. It exists as a function so the
/// off-by-one lives somewhere a test can reach it.
///
/// **Splice, never rebuild.** `ListState::reset` throws away every measured row height and the
/// scroll position with them; a splice keeps both for every row outside the range, which on a
/// one-paragraph edit is every row but one.
pub fn splice_range(delta: BlockDelta) -> (Range<usize>, usize) {
    (
        delta.first_changed..delta.first_changed + delta.old_len,
        delta.new_len,
    )
}

/// Where the viewport's anchor ends up after a reparse's splice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CarriedAnchor {
    /// The anchor in the **new** block table.
    pub at: BlockPoint,
    /// Whether the caller still has to write the scroll.
    ///
    /// `false` for the common case: `ListState::splice` shifts its own `logical_scroll_top` for a
    /// range that ends before the anchor and leaves it alone for one that starts after, so the
    /// list is already showing [`Self::at`] and writing again would only be noise the scroll link
    /// has to absorb.
    pub rewrite: bool,
}

/// **Carry the viewport across an edit.** Given where the viewport sat in the old block table and
/// what the reparse changed, where it sits in the new one.
///
/// Three cases, and only the third costs anything:
///
/// - **The anchor is above the edit.** Same index, same fraction — the user typed below what they
///   are looking at and nothing should move.
/// - **The anchor is below the edit.** The index shifts by `new_len - old_len`; it is still the
///   same block, because everything after the changed range hashed equal. `splice` applies exactly
///   this shift itself, hence `rewrite: false`.
/// - **The anchor is inside the changed range.** The block it named may not exist any more, and
///   `splice` has already snapped the list to the range's first row at offset zero. The anchor is
///   clamped into the new range and its fraction kept, which is as close to "the same place" as a
///   hash diff can promise — this is the one case the caller writes back.
///
/// `None` for an empty table: there is no row to be anchored to.
pub fn carry_anchor(
    anchor: BlockPoint,
    delta: BlockDelta,
    new_block_count: usize,
) -> Option<CarriedAnchor> {
    let last = new_block_count.checked_sub(1)?;
    let changed = delta.first_changed..delta.first_changed + delta.old_len;

    if anchor.block < changed.start {
        return Some(CarriedAnchor {
            at: anchor,
            rewrite: false,
        });
    }
    if anchor.block >= changed.end {
        let shifted = (anchor.block + delta.new_len).saturating_sub(delta.old_len);
        return Some(CarriedAnchor {
            at: BlockPoint::new(shifted.min(last), anchor.fraction),
            rewrite: false,
        });
    }

    // Inside. The last row the splice produced, or the row before it if the edit deleted the whole
    // range and left nothing in its place.
    let last_new = (delta.first_changed + delta.new_len)
        .saturating_sub(1)
        .min(last);
    Some(CarriedAnchor {
        at: BlockPoint::new(anchor.block.min(last_new), anchor.fraction),
        rewrite: true,
    })
}

/// Which half of the split a scroll gesture belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Half {
    /// The text editor on the left.
    Source,
    /// The block list on the right.
    Preview,
}

/// **The thing that stops the echo.** One flag, held while a programmatic scroll is being written.
///
/// The failure it prevents is not subtle. Scrolling the preview writes a scroll offset into the
/// editor; the editor notifies; our observer wakes and reads the editor's new position; that
/// position maps back to a block and a fraction that differ from the originals by a rounding error,
/// and the preview is written to. Left alone the pair converges slowly or oscillates, and either
/// way the user's gesture fights a correction.
///
/// **The driver is the side the gesture arrived at, and nothing infers it.** [`Self::begin`] is
/// called by whichever handler the platform woke; it succeeds, and the write it guards cannot wake
/// the other handler into writing back. Comparing how far each side moved — the base's approach —
/// is exactly what this replaces.
///
/// A `Cell` because both sides reach the link through `&MdView` on the render and handler paths,
/// and taking the gesture is not a state change any observer should see.
#[derive(Debug, Default)]
pub struct ScrollLink {
    driving: Cell<Option<Half>>,
}

impl ScrollLink {
    /// Claim the gesture for `half`, or refuse because the other half is mid-write.
    ///
    /// The refusal *is* the echo suppression: the returned `None` is a handler that was woken by
    /// our own write and should do nothing. Re-entering the same half is refused too — a scroll
    /// handler cannot be its own cause, so a second claim can only be a nested wake.
    pub fn begin(&self, half: Half) -> Option<LinkGuard<'_>> {
        if self.driving.get().is_some() {
            return None;
        }
        self.driving.set(Some(half));
        Some(LinkGuard { link: self })
    }

    /// Claim the link for a **reparse's** anchor rewrite.
    ///
    /// A reparse is not a gesture — nobody scrolled, and the splice that precedes this is not
    /// optional and is not guarded. What *is* a scroll like any other is the write that follows
    /// it: [`carry_anchor`] can hand back a position that has to be pushed into the preview, and
    /// that write would otherwise wake the source half exactly as a wheel gesture's would. So it
    /// is claimed as [`Half::Preview`] — the half it writes into, for the reason a preview gesture
    /// claims it.
    ///
    /// `None` means a real gesture is mid-flight. The caller has already spliced (it must) and
    /// simply leaves the anchor where `ListState::splice` put it, rather than landing a scroll on
    /// top of the one the user is making.
    pub fn begin_reparse(&self) -> Option<LinkGuard<'_>> {
        self.begin(Half::Preview)
    }
}

/// The claim on the link, released when it drops.
///
/// RAII rather than a matched `end()` call because the guarded write has half a dozen early
/// returns in it — an unmeasured row, a gap with no block, a buffer with no layout yet — and one
/// missed `end()` would wedge the link shut for the rest of the session.
pub struct LinkGuard<'a> {
    link: &'a ScrollLink,
}

impl Drop for LinkGuard<'_> {
    fn drop(&mut self) {
        self.link.driving.set(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three blocks with a link reference definition between the second and third — the gap that
    /// `block_at_offset` answers `None` for, and the reason this module never calls it.
    const SAMPLE: &str = "\
# Title

A paragraph of prose that runs on a little.

[ref]: https://example.com

Another paragraph after the gap.
";

    fn sample() -> Document {
        ubiq_md::parse(SAMPLE)
    }

    /// A document whose middle block is a code fence long enough to be taller than any viewport.
    fn tall() -> Document {
        let mut source = String::from("# Title\n\n```rust\n");
        for line in 0..200 {
            source.push_str(&format!("let x{line} = {line};\n"));
        }
        source.push_str("```\n\nAfter.\n");
        ubiq_md::parse(&source)
    }

    #[test]
    fn round_trips_through_every_block() {
        let doc = sample();
        assert!(doc.blocks.len() >= 3, "sample parsed to too few blocks");

        for block in &doc.blocks {
            for step in 0..=8 {
                let fraction = step as f32 / 8.0;
                let offset =
                    block_and_fraction_to_offset(&doc, BlockPoint::new(block.ix, fraction))
                        .expect("every block has a range");
                assert!(
                    block.range.contains(&offset) || offset == block.range.end,
                    "block {} fraction {fraction} left its range: {offset} not in {:?}",
                    block.ix,
                    block.range
                );

                // The other direction has to name the same block and, within a byte of rounding,
                // the same place in it.
                let back =
                    offset_to_block_and_fraction(&doc, offset).expect("offset is in a block");
                assert_eq!(
                    back.block, block.ix,
                    "offset {offset} from block {} came back as block {}",
                    block.ix, back.block
                );

                let again = block_and_fraction_to_offset(&doc, back).expect("round trip");
                assert_eq!(
                    again, offset,
                    "block {} fraction {fraction} did not round trip",
                    block.ix
                );
            }
        }
    }

    #[test]
    fn a_gap_between_blocks_snaps_backwards() {
        let doc = sample();

        // The byte just after some block's last one, which is inside the blank line that follows
        // it — and, for the block before `[ref]:`, inside a stretch the parser produced no block
        // for at all.
        let after_first = doc.blocks[0].range.end;
        assert_eq!(
            doc.block_at_offset(after_first),
            None,
            "the fixture stopped having a gap after block 0"
        );

        let point = offset_to_block_and_fraction(&doc, after_first)
            .expect("a gap resolves to the block before it");
        assert_eq!(point.block, 0);
        assert_eq!(
            point.fraction, 1.0,
            "a gap sits at the foot of the block above"
        );

        // Deep inside the link-reference gap: still the block above, still pinned at its foot,
        // and never `None`.
        let ref_at = SAMPLE.find("[ref]:").expect("fixture has a link reference");
        let deep = offset_to_block_and_fraction(&doc, ref_at + 3).expect("the gap still answers");
        let above = doc
            .block_at_or_before_offset(ref_at + 3)
            .expect("there is a block above the gap");
        assert_eq!(deep.block, above);
        assert_eq!(deep.fraction, 1.0);

        // And an offset before the first block is the only `None` — the empty document.
        assert_eq!(offset_to_block_and_fraction(&ubiq_md::parse(""), 0), None);
    }

    #[test]
    fn a_block_taller_than_the_viewport_still_interpolates() {
        let doc = tall();
        let fence = doc
            .blocks
            .iter()
            .find(|b| matches!(b.kind, ubiq_md::BlockKind::Code { .. }))
            .expect("the fixture has a code fence");

        // The row is far taller than any viewport. The fraction is of the *row*, so every step
        // down it has to name a distinct source offset and a distinct pixel.
        let row_height = 4200.0;
        let viewport = 800.0;
        assert!(row_height > viewport);

        let mut offsets = Vec::new();
        let mut pixels = Vec::new();
        for step in 0..=20 {
            let offset_in_row = row_height * step as f32 / 20.0;
            let fraction = fraction_in_row(offset_in_row, row_height);
            let point = BlockPoint::new(fence.ix, fraction);

            offsets.push(block_and_fraction_to_offset(&doc, point).expect("in range"));
            pixels.push(pixels_in_row(fraction, row_height));
        }

        // Strictly increasing on both sides: no snapping at the block's boundaries, which is the
        // whole complaint about index-only linking.
        for pair in offsets.windows(2) {
            assert!(
                pair[1] > pair[0],
                "the fence snapped instead of scrolling: {pair:?}"
            );
        }
        for pair in pixels.windows(2) {
            assert!(pair[1] > pair[0], "pixels into the row did not advance");
        }
        assert_eq!(offsets.first(), Some(&fence.range.start));
        assert_eq!(offsets.last(), Some(&fence.range.end));

        // And the source line moves with it — the preview → source step ends in a line number.
        let first_line = line_of_offset(&doc.source, offsets[0]);
        let last_line = line_of_offset(&doc.source, *offsets.last().unwrap());
        assert!(
            last_line > first_line + 100,
            "a 200-line fence spanned only {} lines",
            last_line - first_line
        );
        assert_eq!(offset_of_line(&doc.source, first_line), offsets[0]);
    }

    /// Twelve distinct paragraphs under a heading. Distinct on purpose: `BlockDelta` diffs on
    /// content hashes alone, so two identical adjacent paragraphs let the prefix or suffix walk
    /// one block further than the edit did, and a fixture that repeats itself is a fixture whose
    /// delta boundary is a coin toss.
    fn paragraphs() -> Document {
        ubiq_md::parse(&paragraph_source("original"))
    }

    /// The same source with paragraph 6's marker word replaced, so exactly one block's text moves.
    fn paragraph_source(word: &str) -> String {
        let mut source = String::from("# Title\n\n");
        for n in 0..12 {
            if n == 6 {
                source.push_str(&format!(
                    "Paragraph {n}, {word} and distinct from its siblings.\n\n"
                ));
            } else {
                source.push_str(&format!(
                    "Paragraph {n}, distinct from all of its siblings.\n\n"
                ));
            }
        }
        source
    }

    #[test]
    fn an_edit_in_the_middle_splices_only_the_rows_it_changed() {
        let mut doc = paragraphs();
        let before = doc.blocks.len();
        let untouched: Vec<u64> = doc.blocks.iter().map(|b| b.hash).collect();

        let delta = doc.reparse(&paragraph_source("edited"));
        let (rows, count) = splice_range(delta);

        // Strictly inside the table: rows above and below the edit are not in the splice at all,
        // which is the entire reason for preferring it to a rebuild.
        assert!(
            rows.start > 0,
            "the prefix above the edit was not recognised"
        );
        assert!(
            rows.end < before,
            "the suffix below the edit was not recognised"
        );
        assert_eq!(doc.blocks.len(), before - rows.len() + count);

        // And every row outside the spliced range is the same block it was, so the heights the
        // list keeps for them are still theirs.
        for (ix, hash) in untouched.iter().enumerate().take(rows.start) {
            assert_eq!(
                doc.blocks[ix].hash, *hash,
                "row {ix} above the splice changed identity"
            );
        }
        for ix in rows.start + count..doc.blocks.len() {
            // The same row's index in the old table, before the splice moved it.
            let was = ix + rows.len() - count;
            assert_eq!(
                doc.blocks[ix].hash, untouched[was],
                "row {ix} below the splice changed identity"
            );
        }
    }

    #[test]
    fn an_edit_inside_one_block_is_one_row_for_one_row() {
        let mut doc = paragraphs();
        let before = doc.blocks.len();

        let delta = doc.reparse(&paragraph_source("edited"));
        assert_eq!(
            delta.old_len, 1,
            "one paragraph changed, one row should splice"
        );
        assert_eq!(delta.new_len, 1);
        assert_eq!(doc.blocks.len(), before, "the table changed length");

        let (rows, count) = splice_range(delta);
        assert_eq!(rows.len(), 1);
        assert_eq!(count, 1);
    }

    #[test]
    fn a_new_heading_grows_the_table_and_the_outline() {
        let mut doc = paragraphs();
        let blocks_before = doc.blocks.len();
        let outline_before = doc.outline().len();

        // A heading inserted between paragraph 6 and paragraph 7.
        let grown = paragraph_source("original")
            .replace("Paragraph 7,", "## A new section\n\nParagraph 7,");
        let delta = doc.reparse(&grown);
        let (rows, count) = splice_range(delta);

        assert_eq!(
            doc.blocks.len(),
            blocks_before + 1,
            "the table did not grow"
        );
        assert_eq!(
            count,
            rows.len() + 1,
            "the splice did not account for the extra row"
        );

        // The outline is derived from the same table each render, so this is the whole of "the
        // outline follows the edit".
        let outline = doc.outline();
        assert_eq!(outline.len(), outline_before + 1);
        assert!(
            outline
                .iter()
                .any(|(_, level, text)| *level == 2 && text == "A new section"),
            "the new heading is not in the outline: {outline:?}"
        );
        // And its block index is a real row of the new table.
        for (ix, _, _) in &outline {
            assert!(*ix < doc.blocks.len());
        }
    }

    #[test]
    fn the_anchor_block_survives_an_edit_above_it() {
        let mut doc = paragraphs();
        // The viewport is parked a third of the way down the last paragraph.
        let anchor = BlockPoint::new(doc.blocks.len() - 1, 0.33);
        let anchored_on = doc.blocks[anchor.block].hash;

        let grown = paragraph_source("original")
            .replace("Paragraph 1,", "## A new section\n\nParagraph 1,");
        let delta = doc.reparse(&grown);

        let carried =
            carry_anchor(anchor, delta, doc.blocks.len()).expect("the table is not empty");
        assert!(
            !carried.rewrite,
            "an edit above the anchor is a shift `ListState::splice` makes itself"
        );
        assert_eq!(
            carried.at.fraction, anchor.fraction,
            "the fraction was lost"
        );
        assert_eq!(
            doc.blocks[carried.at.block].hash, anchored_on,
            "the anchor came back pointing at a different block"
        );

        // An edit *below* the anchor leaves it exactly where it was.
        let mut doc = paragraphs();
        let anchor = BlockPoint::new(1, 0.5);
        let anchored_on = doc.blocks[anchor.block].hash;
        let delta = doc.reparse(&paragraph_source("edited"));
        let carried = carry_anchor(anchor, delta, doc.blocks.len()).expect("not empty");
        assert!(!carried.rewrite);
        assert_eq!(carried.at, anchor);
        assert_eq!(doc.blocks[carried.at.block].hash, anchored_on);

        // And an anchor *inside* the changed range is the one case the caller writes back: the
        // block it named may be gone, so it is clamped into the new range with its fraction kept.
        let mut doc = paragraphs();
        let delta = doc.reparse(&paragraph_source("edited"));
        let inside = BlockPoint::new(delta.first_changed, 0.75);
        let carried = carry_anchor(inside, delta, doc.blocks.len()).expect("not empty");
        assert!(
            carried.rewrite,
            "an anchor inside the splice needs rewriting"
        );
        assert_eq!(carried.at.fraction, 0.75);
        assert!(carried.at.block < doc.blocks.len());

        // The empty table is the only `None` — a document edited down to nothing.
        let mut doc = paragraphs();
        let delta = doc.reparse("");
        assert_eq!(doc.blocks.len(), 0);
        assert_eq!(carry_anchor(BlockPoint::new(3, 0.5), delta, 0), None);
    }

    #[test]
    fn the_guard_suppresses_the_echo() {
        let link = ScrollLink::default();

        // The preview took the gesture.
        let guard = link
            .begin(Half::Preview)
            .expect("an idle link grants the gesture");

        // The write into the editor wakes the source handler. It must decline — this is the echo.
        assert!(
            link.begin(Half::Source).is_none(),
            "the source half echoed the preview's write"
        );
        // A nested wake of the same half declines too.
        assert!(link.begin(Half::Preview).is_none());

        drop(guard);

        // The next real gesture, from either side, is granted.
        let guard = link
            .begin(Half::Source)
            .expect("the link reopens once the write is done");
        assert!(link.begin(Half::Preview).is_none());
        drop(guard);
        assert!(link.begin(Half::Preview).is_some());

        // A reparse's anchor rewrite plays by the same rules: it declines while a gesture is in
        // flight — the splice has happened regardless — and, once granted, is not echoed back.
        let guard = link.begin(Half::Source).expect("idle");
        assert!(
            link.begin_reparse().is_none(),
            "a reparse landed a scroll on top of a live gesture"
        );
        drop(guard);
        let guard = link
            .begin_reparse()
            .expect("an idle link grants the rewrite");
        assert!(link.begin(Half::Source).is_none());
        drop(guard);
    }
}
