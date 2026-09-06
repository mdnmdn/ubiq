//! What the index will not do. Every bound here is a number the reply can carry, never a silent
//! stop — a search that hit one says so, exactly as the walk's own ceilings do.

use std::time::Duration;

/// The schema and tokenizer this build writes and understands.
///
/// Bumping it is how a schema or tokenizer change ships: a directory stamped with anything else is
/// deleted and rebuilt rather than migrated, because the index is derived data and rebuilding it
/// costs a walk.
pub const STAMP: u32 = 1;

/// Files above this are never indexed. They are still found by the walk, and by nothing else.
///
/// A megabyte of source is already pathological; what this really excludes is generated files,
/// vendored bundles and data checked in beside code, all of which cost far more to tokenise than
/// anyone gains from finding them.
pub const MAX_FILE_BYTES: u64 = 1 << 20;

/// How much of a file's head is read to decide it is binary. `grep-searcher` makes the same call
/// on the read side; this one only keeps the index from storing what could never be shown.
pub const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// Queries shorter than one trigram have nothing to look up, so they take the walk.
pub const MIN_QUERY_CHARS: usize = 3;

/// How many candidate paths one query resolves to. Past it the reply is `truncated` — the same
/// flag the walk's own ceilings set, meaning the same thing.
pub const CANDIDATES: usize = 2_000;

/// How many files one project's index holds. Past it the rest of the project is not indexed and
/// is found by the walk alone — which is why the walk is never removed.
pub const FILES: usize = 50_000;

/// The writer's arena, per open project. One indexing thread each, so two open projects cannot
/// spawn a thread storm against the walk they share a machine with.
pub const WRITER_HEAP: usize = 15_000_000;

/// How long the queue must be idle before a commit. A commit is the expensive part of an
/// incremental update, and an editor saving a file produces a burst, not one event.
pub const COMMIT_QUIET: Duration = Duration::from_secs(1);

/// How many pending paths force a commit regardless of quiet. A bulk change — a branch switch, a
/// generated directory landing — must not sit uncommitted until the user stops working.
pub const COMMIT_PENDING: usize = 512;
