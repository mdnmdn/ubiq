//! Ceilings for a project search. Part of the contract: every ceiling that bites is drawn.

// The most matches kept per file. Past this the file's [`FileHit::truncated`] is set.
pub const HITS_PER_FILE: usize = 100;

// The most files that contribute hits. Past this the search stops and the overall
// [`SearchFinished::truncated`] is set.
pub const FILES_WITH_HITS: usize = 1_000;

// The most matches kept overall. Past this the search stops.
pub const TOTAL_HITS: usize = 10_000;

// How many files the walker sees between progress reports.
pub const PROGRESS_INTERVAL: usize = 100;
