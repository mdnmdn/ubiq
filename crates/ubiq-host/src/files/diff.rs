//! One file's change against a version-control base.
//!
//! **A diff is not a file.** Its content is a comparison, and computing one means reading version
//! control — which is the host's, exactly as reading a directory is. The host answers hunks with
//! their line numbers already worked out, so no diff library ever reaches the interface, on the
//! same discipline that keeps a VT parser out of the host.
//!
//! It runs where the rest of the family runs: on the `ubiq-files` worker, never on the
//! coordinator's thread. Opening a repository, walking a tree and inflating a blob are all
//! syscalls, and a cold `.git` on a network mount would stall every pane behind it.
//!
//! Both sides are compared as **raw bytes as they are stored** — the blob as it sits in the object
//! database against the file as it sits on disk. Git's own clean and smudge filters are not run,
//! because running them means running configured programs from a folder the user merely opened.

use std::fs;
use std::io::Read;
use std::path::Path;

use git2::{ErrorCode, ObjectType, Oid, Repository};
use similar::{ChangeTag, TextDiff};
use ubiq_proto::files::{DiffBase, DiffHunk, DiffRow, DiffRowKind, FileDiff, FileError};

use super::path;
use super::{MAX_READ_BYTES, from_io, looks_binary};

/// How much of either side the host will compare.
///
/// The same ceiling a read has, and for the same reason: this is answered because somebody clicked.
/// Past it the diff comes back `truncated` with no hunks rather than as a comparison of two
/// prefixes, which would draw the tail of the longer side as one enormous deletion that never
/// happened.
const MAX_SIDE_BYTES: u64 = MAX_READ_BYTES;

/// How many hunks one answer carries.
const MAX_HUNKS: usize = 400;

/// How many rows one answer carries, across all its hunks.
const MAX_ROWS: usize = 10_000;

/// Unchanged lines kept on either side of a run of changed ones. Three, as a unified diff has.
const CONTEXT: usize = 3;

/// Compare the working tree's copy of `rel_path` with `base`.
///
/// A file with no change answers with no hunks and no error. A file version control has never seen
/// — untracked, or ignored — has no blob on either base, and is answered as wholly added: that is
/// what the working tree actually adds against the base, and it is what an editor's gutter shows.
pub fn diff(root: &Path, rel_path: &str, base: DiffBase) -> Result<FileDiff, FileError> {
    let file = path::resolve(root, rel_path)?;
    let stat = fs::metadata(&file).map_err(from_io)?;
    if !stat.is_file() {
        return Err(FileError::WrongKind);
    }

    // Upwards from the project's root, so a project that is a folder inside a repository is
    // diffed against that repository. A project with none above it is refused — the host says
    // there is no version control here rather than drawing an empty diff, which would read as a
    // file with no changes.
    let repo = Repository::discover(root).map_err(|error| {
        FileError::Refused(format!("no version control here: {}", error.message()))
    })?;
    let Some(workdir) = repo.workdir().map(Path::to_path_buf) else {
        return Err(FileError::Refused(
            "a bare repository has no working tree to compare".to_string(),
        ));
    };
    // Canonical on both sides, because the resolved file is canonical and a repository discovered
    // through a symlinked temporary directory is not.
    let workdir = fs::canonicalize(&workdir).map_err(from_io)?;
    let tracked = file.strip_prefix(&workdir).map_err(|_| {
        FileError::Refused("the file is outside the repository's working tree".to_string())
    })?;

    let old = match base {
        DiffBase::Head => head_blob(&repo, tracked)?,
        DiffBase::Index => index_blob(&repo, tracked)?,
    }
    .unwrap_or_default();
    let (new, over_ceiling) = working_tree(&file)?;

    if looks_binary(&old) || looks_binary(&new) {
        return Ok(FileDiff {
            base,
            hunks: Vec::new(),
            binary: true,
            truncated: false,
        });
    }
    if over_ceiling || old.len() as u64 > MAX_SIDE_BYTES {
        return Ok(FileDiff {
            base,
            hunks: Vec::new(),
            binary: false,
            truncated: true,
        });
    }

    // Lossy is safe here and only here: a side holding a NUL has already left as `binary`, and
    // what is left is text with at worst a severed sequence in it, which is the interface's
    // problem to draw rather than a reason to refuse the whole comparison.
    let old = String::from_utf8_lossy(&old);
    let new = String::from_utf8_lossy(&new);
    let (hunks, truncated) = hunks(&old, &new);

    Ok(FileDiff {
        base,
        hunks,
        binary: false,
        truncated,
    })
}

/// The blob the checked-out commit holds for this path, if it holds one.
fn head_blob(repo: &Repository, tracked: &Path) -> Result<Option<Vec<u8>>, FileError> {
    let head = match repo.head() {
        Ok(head) => head,
        // A repository whose first commit has not been made. Nothing has a blob yet, so every
        // file in it is wholly added — the same answer an untracked file gets.
        Err(error) if unborn(&error) => return Ok(None),
        Err(error) => return Err(failed(error)),
    };
    let tree = head.peel_to_tree().map_err(failed)?;
    match tree.get_path(tracked) {
        Ok(entry) if entry.kind() == Some(ObjectType::Blob) => blob(repo, entry.id()).map(Some),
        // A path that is a file on disk and a directory or a submodule in the commit. There is no
        // old side to show, so the working tree's copy is what was added.
        Ok(_) => Ok(None),
        Err(error) if error.code() == ErrorCode::NotFound => Ok(None),
        Err(error) => Err(failed(error)),
    }
}

/// The blob the index holds for this path at stage zero, if it holds one.
///
/// Only stage zero: a path in the middle of a merge has no single staged version, and answering
/// with one side of a conflict would be a comparison against something that is not there.
fn index_blob(repo: &Repository, tracked: &Path) -> Result<Option<Vec<u8>>, FileError> {
    let index = repo.index().map_err(failed)?;
    match index.get_path(tracked, 0) {
        Some(entry) => blob(repo, entry.id).map(Some),
        None => Ok(None),
    }
}

/// One object's bytes.
fn blob(repo: &Repository, id: Oid) -> Result<Vec<u8>, FileError> {
    let blob = repo.find_blob(id).map_err(failed)?;
    Ok(blob.content().to_vec())
}

/// The file as it is on disk, and whether it is past the ceiling.
///
/// One byte past, so the answer comes from what was read rather than from a stat taken before it.
fn working_tree(file: &Path) -> Result<(Vec<u8>, bool), FileError> {
    let handle = fs::File::open(file).map_err(from_io)?;
    let mut bytes = Vec::new();
    handle
        .take(MAX_SIDE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(from_io)?;
    let over = bytes.len() as u64 > MAX_SIDE_BYTES;
    Ok((bytes, over))
}

/// Walk the change and build the hunks, stopping at the ceilings.
///
/// The second value is whether a ceiling stopped it, which is `FileDiff::truncated`: the hunks are
/// then a prefix of the change rather than all of it, and saying so is the difference between a
/// short answer and a lying one.
fn hunks(old: &str, new: &str) -> (Vec<DiffHunk>, bool) {
    let comparison = TextDiff::from_lines(old, new);
    let groups = comparison.grouped_ops(CONTEXT);

    let mut hunks = Vec::new();
    let mut rows_so_far = 0usize;

    for group in &groups {
        // `grouped_ops` never yields an empty group, so both ends exist.
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        if hunks.len() >= MAX_HUNKS || rows_so_far >= MAX_ROWS {
            return (hunks, true);
        }

        let mut rows = Vec::new();
        let mut cut = false;
        for op in group {
            for change in comparison.iter_changes(op) {
                if rows_so_far + rows.len() >= MAX_ROWS {
                    cut = true;
                    break;
                }
                let kind = match change.tag() {
                    ChangeTag::Equal => DiffRowKind::Context,
                    ChangeTag::Delete => DiffRowKind::Removed,
                    ChangeTag::Insert => DiffRowKind::Added,
                };
                rows.push(DiffRow {
                    kind,
                    old_line: change.old_index().map(one_based),
                    new_line: change.new_index().map(one_based),
                    text: line(change.value()),
                });
            }
            if cut {
                break;
            }
        }

        rows_so_far += rows.len();
        let old_range = first.old_range().start..last.old_range().end;
        let new_range = first.new_range().start..last.new_range().end;
        hunks.push(DiffHunk {
            old_start: start(old_range.start, old_range.len()),
            old_lines: old_range.len() as u32,
            new_start: start(new_range.start, new_range.len()),
            new_lines: new_range.len() as u32,
            rows,
        });
        if cut {
            return (hunks, true);
        }
    }

    (hunks, false)
}

/// A hunk's first line on one side.
///
/// One-based, except that a side the hunk covers no line of starts at the line before it — which is
/// what a unified diff's `@@ -0,0 +1,3 @@` says about a file that did not exist.
fn start(index: usize, lines: usize) -> u32 {
    let start = if lines == 0 { index } else { index + 1 };
    start as u32
}

/// A zero-based index as the contract's one-based line number.
fn one_based(index: usize) -> u32 {
    (index as u32).saturating_add(1)
}

/// One line, without the terminator the split kept on it.
///
/// The marker a textual diff would put at the front is never here: it is [`DiffRow::kind`], a thing
/// to draw rather than a character to strip.
fn line(value: &str) -> String {
    let value = value.strip_suffix('\n').unwrap_or(value);
    let value = value.strip_suffix('\r').unwrap_or(value);
    value.to_string()
}

/// Whether the repository simply has no commit yet.
fn unborn(error: &git2::Error) -> bool {
    matches!(error.code(), ErrorCode::UnbornBranch | ErrorCode::NotFound)
}

/// Version control's refusal as the contract's.
///
/// `Failed`, not `Refused`: a corrupt object or an unreadable `.git` is the state of the folder,
/// not a wiring mistake, and only the missing-repository case is a refusal.
fn failed(error: git2::Error) -> FileError {
    FileError::Failed(error.message().to_string())
}
