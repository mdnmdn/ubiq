//! Producing hits from one file, and accumulating them into batches.
//!
//! Split out of [`super::worker`] because a hit is produced the same way however the file was
//! chosen. The walk finds files by visiting all of them; an index finds them by looking them up.
//! Both then read the same bytes through the same `grep-searcher` sink under the same ceilings,
//! so a hit is byte-identical either way and the interface cannot tell which path ran.

use std::path::Path;
use std::time::Instant;

use grep_matcher::Matcher;
use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use ubiq_proto::search::{Batch, FileHit, LineHit};

use super::ceiling;

/// Search one file and answer its hits, or `None` when it had none.
///
/// Binary detection and line reading are the searcher's; the per-file ceiling stops it early. A
/// file that cannot be read answers `None` — unreadable and unmatched are the same to a caller
/// that is about to move on to the next one either way.
pub fn scan_file(
    matcher: &grep_regex::RegexMatcher,
    abs_path: &Path,
    rel_path: String,
) -> Option<FileHit> {
    let mut lines: Vec<LineHit> = Vec::new();
    let mut searcher = SearcherBuilder::new()
        .line_number(true)
        .binary_detection(BinaryDetection::quit(0))
        .build();

    let sink = UTF8(
        |line_number: u64, line_text: &str| -> Result<bool, std::io::Error> {
            let ranges = match_ranges(matcher, line_text);
            if !ranges.is_empty() {
                // The searcher hands us the line with its terminator still on: a `LineHit` is
                // the line, not the line plus the byte that ends it, so drop it here rather than
                // carrying it into a row the interface has to draw as one line.
                lines.push(LineHit {
                    line: line_number as u32,
                    text: line_text.trim_end_matches(['\n', '\r']).to_string(),
                    ranges,
                });
            }
            // Stop the searcher early if the per-file ceiling is hit.
            Ok(lines.len() < ceiling::HITS_PER_FILE)
        },
    );

    let _ = searcher.search_path(matcher, abs_path, sink);

    if lines.is_empty() {
        return None;
    }
    let truncated = lines.len() >= ceiling::HITS_PER_FILE;
    Some(FileHit {
        rel_path,
        lines,
        truncated,
    })
}

/// Byte-offset highlight ranges for a matched line, by re-running the matcher over it.
fn match_ranges(matcher: &grep_regex::RegexMatcher, line_text: &str) -> Vec<(u32, u32)> {
    let mut ranges = Vec::new();
    let mut last_end = 0usize;
    let _ = matcher.find_iter(line_text.as_bytes(), |m| {
        let start = m.start() as u32;
        let end = m.end() as u32;
        if (start as usize) >= last_end {
            ranges.push((start, end));
            last_end = m.end();
        }
        true
    });
    ranges
}

/// What one search has accumulated so far: the open batch, and the counts every ceiling reads.
///
/// Shared across the parallel walk behind a mutex, and touched only between files.
pub struct State {
    batch_files: Vec<FileHit>,
    /// Every file the search looked at, hits or not — what `SearchProgress` reports.
    pub files_seen: usize,
    /// Files that contributed at least one hit — what `FILES_WITH_HITS` bounds.
    pub files_with_hits: usize,
    pub total_hits: usize,
    /// Accumulated hits since last flush.
    pending_hits: usize,
    /// When the current batch started, so a slow trickle is still flushed.
    opened_at: Instant,
    /// Progress last reported at this `files_seen`.
    pub reported_at: usize,
}

impl State {
    pub fn new() -> Self {
        Self {
            batch_files: Vec::new(),
            files_seen: 0,
            files_with_hits: 0,
            total_hits: 0,
            pending_hits: 0,
            opened_at: Instant::now(),
            reported_at: 0,
        }
    }

    /// A file the search looked at, whether or not it had hits.
    pub fn saw_file(&mut self) {
        self.files_seen += 1;
    }

    pub fn add_file(&mut self, hit: FileHit) {
        self.files_with_hits += 1;
        self.pending_hits += hit.lines.len();
        self.total_hits += hit.lines.len();
        self.batch_files.push(hit);
    }

    pub fn should_flush(&self) -> bool {
        !self.batch_files.is_empty()
            && (self.batch_files.len() >= ceiling::BATCH_FILES
                || self.pending_hits >= ceiling::BATCH_HITS
                || self.opened_at.elapsed() >= ceiling::BATCH_INTERVAL)
    }

    pub fn take_batch(&mut self) -> Batch {
        self.pending_hits = 0;
        self.opened_at = Instant::now();
        Batch::Files(std::mem::take(&mut self.batch_files))
    }

    pub fn is_empty(&self) -> bool {
        self.batch_files.is_empty()
    }

    /// Whether a whole-search ceiling has been reached, which is what stops the search and sets
    /// `truncated` on the reply.
    pub fn at_ceiling(&self) -> bool {
        self.total_hits >= ceiling::TOTAL_HITS || self.files_with_hits >= ceiling::FILES_WITH_HITS
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// `grep-searcher` hands the sink each line with its terminator still attached — verified
    /// directly against the crate, not assumed. A `LineHit` is the line the interface draws as
    /// one row; a stray `\n` reaching `StyledText` shapes as a second, empty line and doubles the
    /// row's height, which is exactly the offset the search panel showed.
    #[test]
    fn a_hit_s_text_carries_no_line_terminator() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "before").unwrap();
        writeln!(file, "the match is here").unwrap();
        writeln!(file, "after").unwrap();
        file.flush().unwrap();

        let matcher = grep_regex::RegexMatcher::new("match").unwrap();
        let hit = scan_file(&matcher, file.path(), "fixture".to_string()).unwrap();

        assert_eq!(hit.lines.len(), 1);
        let text = &hit.lines[0].text;
        assert_eq!(text, "the match is here");
        assert!(!text.ends_with('\n') && !text.ends_with('\r'));
    }
}
