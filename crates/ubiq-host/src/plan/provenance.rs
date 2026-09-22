//! Who last wrote each line of a plan, and what the saves that got it there did.
//!
//! An agent that wrote a plan and comes back to it needs one answer: *where has a human been
//! since, and how much did they change?* The annotation layer cannot give it — a block a human
//! reworded is the same block with the same id, and the matcher's whole job is to keep it that
//! way — so this is a second layer over the same sidecar, and it counts lines rather than blocks.
//!
//! **The stored form is a stamp per line of the current body, kept as runs.**
//!
//! - A [`Stamp`] is the revision and the [`SaveOrigin`] that last rewrote one line.
//! - On each save, [`advance`] walks the new body once: a line the diff touched takes the new
//!   save's stamp, and a line that merely moved carries its old stamp along
//!   [`super::lines::LineDiff::source`]. That is the accumulation — the layer never re-derives
//!   history, it rebases one stamp per line through each save as it happens, which is the only
//!   form that stays correct when a later edit renumbers everything below it.
//! - [`compress`] stores the result as [`ProvenanceRun`]s rather than one JSON object per line.
//!   Same information, and a sidecar a person can still read by hand — which is the reason
//!   `store/plan.rs` gives for the file being JSON in the first place.
//!
//! **An unstamped line is not a changed line.** A sidecar written before this layer existed has
//! no runs, and the lines it describes carry no stamp; they stay unstamped rather than being
//! claimed by whoever saves next. So an existing plan loads, keeps its annotations, and reports
//! honestly that nothing is *known* to have changed — rather than telling an agent a human just
//! rewrote a document they never touched.
//!
//! **[`RevisionEntry`] is the other half**, and it is per save rather than per line: the counts a
//! line stamp cannot hold, because a line rewritten twice is one stamp and two modifications.
//! The stats come from here and the regions come from the stamps, which is why the two answer
//! slightly different questions on purpose.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ubiq_proto::ids::BlockId;
use ubiq_proto::plan::{PlanBlock, PlanChangeStats, PlanChangedRegion, PlanRevision, SaveOrigin};

use super::lines::LineDiff;

/// How many saves of per-revision counts a sidecar keeps.
///
/// The stamps are what answer "where", and they are complete for all time; this list only feeds
/// the counts, so trimming it costs the exact tally for a watermark older than the last five
/// hundred saves and nothing else. Unbounded, it would grow forever on a document that is saved
/// on a timer.
pub const HISTORY_LIMIT: usize = 500;

/// The revision and origin that last rewrote one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub revision: PlanRevision,
    pub origin: SaveOrigin,
}

/// A contiguous run of lines sharing one stamp — the stored form of the per-line provenance.
///
/// 1-based and inclusive at both ends, the numbering [`PlanChangedRegion`] uses, so the wire form
/// is this plus the text and the block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceRun {
    pub first_line: u32,
    pub last_line: u32,
    pub revision: PlanRevision,
    pub origin: SaveOrigin,
}

/// What one save did, as a count. One per revision, newest last.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionEntry {
    pub revision: PlanRevision,
    pub origin: SaveOrigin,
    /// Which agent, for a save an agent made — [`crate::mcp::registry::AgentFacts::key`], the
    /// same string the run directory is named with. `None` for every human save: the interface
    /// does not identify *which* person, and there is only ever the one at the keyboard.
    ///
    /// This is what lets `plan_changes` default its watermark to the calling agent's own last
    /// `write_plan` rather than to some other agent's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub at: DateTime<Utc>,
    #[serde(default)]
    pub added: u32,
    #[serde(default)]
    pub removed: u32,
    #[serde(default)]
    pub modified: u32,
}

impl RevisionEntry {
    /// Whether this save changed nothing in the body. A save that rewrites a file with the bytes
    /// already in it still takes a revision — the counter follows saves, not content — and this
    /// is how a reader tells the two apart.
    pub fn is_empty(&self) -> bool {
        self.added == 0 && self.removed == 0 && self.modified == 0
    }
}

/// Bring the previous save's runs forward onto the new body, stamping what this save changed.
///
/// The one place the accumulated layer is written. Everything else here reads it.
pub fn advance(
    previous: &[ProvenanceRun],
    diff: &LineDiff,
    revision: PlanRevision,
    origin: SaveOrigin,
) -> Vec<ProvenanceRun> {
    let before = expand(previous);
    let stamp = Stamp { revision, origin };

    let stamps: Vec<Option<Stamp>> = (0..diff.touched.len())
        .map(|line| {
            if diff.touched[line] {
                return Some(stamp);
            }
            // Untouched: it carries whatever it already had, including nothing. Never the new
            // stamp — this save did not write it.
            diff.source[line].and_then(|from| before.get(from).copied().flatten())
        })
        .collect();

    compress(&stamps)
}

/// The runs as one entry per line, indexed from zero — the form [`advance`] looks a line up in.
/// Lines past the last run, and lines no run covers, come back as `None`.
fn expand(runs: &[ProvenanceRun]) -> Vec<Option<Stamp>> {
    let lines = runs
        .iter()
        .map(|run| run.last_line as usize)
        .max()
        .unwrap_or(0);
    let mut out = vec![None; lines];
    for run in runs {
        // 1-based on the wire and on disk, 0-based in the vector.
        let first = run.first_line.saturating_sub(1) as usize;
        let last = run.last_line.saturating_sub(1) as usize;
        for slot in out.iter_mut().take(last + 1).skip(first) {
            *slot = Some(Stamp {
                revision: run.revision,
                origin: run.origin,
            });
        }
    }
    out
}

/// Group a per-line stamp vector back into runs. A run breaks wherever the revision or the origin
/// changes, or wherever a line has no stamp at all, so every run describes all of its lines
/// exactly.
fn compress(stamps: &[Option<Stamp>]) -> Vec<ProvenanceRun> {
    let mut out: Vec<ProvenanceRun> = Vec::new();
    for (index, stamp) in stamps.iter().enumerate() {
        let Some(stamp) = stamp else { continue };
        let line = index as u32 + 1;
        match out.last_mut() {
            Some(run)
                if run.last_line + 1 == line
                    && run.revision == stamp.revision
                    && run.origin == stamp.origin =>
            {
                run.last_line = line;
            }
            _ => out.push(ProvenanceRun {
                first_line: line,
                last_line: line,
                revision: stamp.revision,
                origin: stamp.origin,
            }),
        }
    }
    out
}

/// The runs stamped after `since`, as regions carrying the text now standing at those lines and
/// the block each falls in.
///
/// In document order, because the runs are. A run at or below the watermark is simply not in the
/// window and is skipped — which is what makes a watermark of the current revision answer
/// nothing at all.
pub fn regions(
    runs: &[ProvenanceRun],
    since: PlanRevision,
    body: &str,
    blocks: &[PlanBlock],
) -> Vec<PlanChangedRegion> {
    let lines = super::lines::split(body);
    let spans = block_spans(body, blocks);

    runs.iter()
        .filter(|run| run.revision > since)
        .map(|run| {
            let first = run.first_line.saturating_sub(1) as usize;
            let last = (run.last_line as usize).min(lines.len());
            let text = lines
                .get(first..last)
                .map(|slice| slice.join("\n"))
                .unwrap_or_default();
            PlanChangedRegion {
                first_line: run.first_line,
                last_line: run.last_line,
                revision: run.revision,
                origin: run.origin,
                block_id: containing_block(&spans, run.first_line, run.last_line),
                text,
            }
        })
        .collect()
}

/// What changed since `since`, counted.
///
/// The line counts come from the history and the block count from the regions — see the module
/// doc for why those are two different readings rather than one derived from the other.
pub fn stats(
    history: &[RevisionEntry],
    since: PlanRevision,
    revision: PlanRevision,
    regions: &[PlanChangedRegion],
) -> PlanChangeStats {
    let mut stats = PlanChangeStats {
        since_revision: since,
        revision,
        ..PlanChangeStats::default()
    };

    for entry in history.iter().filter(|entry| entry.revision > since) {
        stats.lines_added += entry.added;
        stats.lines_removed += entry.removed;
        stats.lines_modified += entry.modified;
        match entry.origin {
            SaveOrigin::Human => stats.human_revisions += 1,
            SaveOrigin::Agent => stats.agent_revisions += 1,
        }
    }

    let touched: HashSet<BlockId> = regions
        .iter()
        .filter_map(|region| region.block_id)
        .collect();
    stats.blocks_touched = touched.len() as u32;

    stats
}

/// Where each indexed block sits in the body, as a 1-based inclusive line span.
///
/// Found by scanning forward for the block's own text, which is what the index stores, rather
/// than re-parsing: the blocks are already in document order and each one's text appears once at
/// or after the previous one's end, so a single cursor walk finds them all. A block whose text no
/// longer appears — the index is a save behind, or the text was normalised — is skipped, and the
/// regions inside it come back with no block. Best effort by design, which is what
/// [`PlanChangedRegion::block_id`] promises.
fn block_spans(body: &str, blocks: &[PlanBlock]) -> Vec<(BlockId, u32, u32)> {
    let starts = line_starts(body);
    let mut out = Vec::with_capacity(blocks.len());
    let mut cursor = 0usize;

    for block in blocks {
        if block.text.is_empty() {
            continue;
        }
        let Some(found) = body.get(cursor..).and_then(|rest| rest.find(&block.text)) else {
            continue;
        };
        let start = cursor + found;
        let end = start + block.text.len();
        out.push((
            block.id,
            line_of(&starts, start),
            line_of(&starts, end.saturating_sub(1)),
        ));
        cursor = end;
    }

    out
}

/// The byte offset each line begins at.
fn line_starts(body: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    starts.extend(
        body.match_indices('\n')
            .map(|(offset, _)| offset + 1)
            .filter(|offset| *offset < body.len()),
    );
    starts
}

/// The 1-based line a byte offset falls on.
fn line_of(starts: &[usize], offset: usize) -> u32 {
    match starts.binary_search(&offset) {
        Ok(line) => line as u32 + 1,
        Err(next) => next as u32,
    }
}

/// The block that contains the whole run, if one does. A run straddling two blocks, or sitting in
/// the blank space between them, has none — the line numbers are the authoritative answer and
/// this is the convenience on top.
fn containing_block(spans: &[(BlockId, u32, u32)], first: u32, last: u32) -> Option<BlockId> {
    spans
        .iter()
        .find(|(_, start, end)| *start <= first && *end >= last)
        .map(|(id, _, _)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::lines;

    fn run(first: u32, last: u32, revision: u64, origin: SaveOrigin) -> ProvenanceRun {
        ProvenanceRun {
            first_line: first,
            last_line: last,
            revision,
            origin,
        }
    }

    #[test]
    fn the_first_save_stamps_every_line() {
        let diff = lines::diff("", "a\nb\nc");
        let runs = advance(&[], &diff, 1, SaveOrigin::Agent);
        assert_eq!(runs, vec![run(1, 3, 1, SaveOrigin::Agent)]);
    }

    #[test]
    fn an_untouched_line_keeps_its_stamp_when_an_edit_above_moves_it() {
        let first = advance(&[], &lines::diff("", "a\nb"), 1, SaveOrigin::Agent);
        // A human inserts a line at the top: "a" and "b" slide down but are still the agent's.
        let second = advance(
            &first,
            &lines::diff("a\nb", "new\na\nb"),
            2,
            SaveOrigin::Human,
        );
        assert_eq!(
            second,
            vec![
                run(1, 1, 2, SaveOrigin::Human),
                run(2, 3, 1, SaveOrigin::Agent),
            ],
        );
    }

    #[test]
    fn a_line_with_no_previous_stamp_is_not_claimed_by_the_next_save() {
        // An older sidecar: a body on disk and no provenance at all.
        let diff = lines::diff("a\nb\nc", "a\nB\nc");
        let runs = advance(&[], &diff, 1, SaveOrigin::Human);
        assert_eq!(
            runs,
            vec![run(2, 2, 1, SaveOrigin::Human)],
            "only the line this save actually rewrote",
        );
    }

    #[test]
    fn a_run_breaks_where_the_origin_changes() {
        let stamps = vec![
            Some(Stamp {
                revision: 1,
                origin: SaveOrigin::Agent,
            }),
            Some(Stamp {
                revision: 1,
                origin: SaveOrigin::Human,
            }),
        ];
        assert_eq!(
            compress(&stamps),
            vec![
                run(1, 1, 1, SaveOrigin::Agent),
                run(2, 2, 1, SaveOrigin::Human),
            ],
        );
    }

    #[test]
    fn runs_round_trip_through_expand_and_compress() {
        let runs = vec![
            run(1, 2, 3, SaveOrigin::Agent),
            run(4, 5, 7, SaveOrigin::Human),
        ];
        // Line 3 has no stamp, so it stays a gap rather than joining either side.
        assert_eq!(compress(&expand(&runs)), runs);
    }

    #[test]
    fn a_region_carries_the_text_now_standing_at_those_lines() {
        let body = "# Plan\n\nStep one.\n\nStep two.";
        let runs = vec![run(5, 5, 2, SaveOrigin::Human)];
        let regions = regions(&runs, 1, body, &[]);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].text, "Step two.");
        assert_eq!(regions[0].origin, SaveOrigin::Human);
    }

    #[test]
    fn a_watermark_at_the_current_revision_reports_nothing() {
        let runs = vec![run(1, 3, 4, SaveOrigin::Human)];
        assert!(regions(&runs, 4, "a\nb\nc", &[]).is_empty());
    }

    #[test]
    fn a_region_names_the_block_it_falls_inside() {
        let body = "# Plan\n\nStep one.\n\nStep two.";
        let blocks = crate::plan::blocks::match_blocks(&[], &crate::plan::blocks::blocks(body));
        let runs = vec![run(5, 5, 2, SaveOrigin::Human)];
        let regions = regions(&runs, 1, body, &blocks.blocks);
        assert_eq!(
            regions[0].block_id,
            Some(blocks.blocks[2].id),
            "line 5 is the third block, 'Step two.'",
        );
    }

    #[test]
    fn the_stats_count_revisions_in_the_window_and_not_before_it() {
        let history = vec![
            RevisionEntry {
                revision: 1,
                origin: SaveOrigin::Agent,
                author: Some("agent-1".to_string()),
                at: Utc::now(),
                added: 10,
                removed: 0,
                modified: 0,
            },
            RevisionEntry {
                revision: 2,
                origin: SaveOrigin::Human,
                author: None,
                at: Utc::now(),
                added: 1,
                removed: 2,
                modified: 3,
            },
        ];
        let stats = stats(&history, 1, 2, &[]);
        assert_eq!(stats.human_revisions, 1);
        assert_eq!(
            stats.agent_revisions, 0,
            "revision 1 is below the watermark"
        );
        assert_eq!(
            (stats.lines_added, stats.lines_removed, stats.lines_modified),
            (1, 2, 3),
        );
    }
}
