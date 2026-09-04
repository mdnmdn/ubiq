//! A project's files as the host reads them: one level of its tree, one file's bytes, one file
//! written back, and one file compared with what version control holds.
//!
//! Everything here is bounded. A directory has an entry ceiling, a read has a byte ceiling, a walk
//! has a depth, and a path has a length — because the interface asks for these by clicking, and a
//! click on a repository's root must not be able to cost more than a frame.
//!
//! **Nothing here runs on the coordinator's thread.** A `read_dir` of a cold directory, a
//! `canonicalize` on a network mount that has not come up, or a two-megabyte read would stall every
//! pane's keystrokes and every resize behind it. That is the dual of the rule that the coordinator's
//! reader is never blocked by a slow window, and [`Files`] is what keeps it: the coordinator takes a
//! record's root from memory, hands over a [`Job`], and answers nothing itself.
//!
//! The path resolution every one of these starts with is [`path`], which is the security boundary.
//! The comparison is [`diff`], which is the only place version control is read.

pub mod diff;
pub mod path;

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::thread;

use ubiq_proto::bus::Mailbox;
use ubiq_proto::files::{
    DiffBase, DirEntry, DirListing, EntryKind, FileContents, FileError, FileVersion, LIST_HIDE,
    PathOp, WALK_SKIP,
};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;

/// One directory's ceiling. Past it the listing says it is truncated.
const MAX_ENTRIES: usize = 2_000;

/// Every entry one reply may carry, across all the listings in it.
const MAX_REPLY_ENTRIES: usize = 10_000;

/// How deep a single request may ask to walk.
///
/// One is what an expand asks for. More is what the window's background cache uses, so a search
/// can see files in folders nobody has opened, still bounded so a click cannot walk the whole of
/// `node_modules`.
const MAX_DEPTH: u8 = 3;

/// The most a read returns unless the interface asks for less.
const MAX_READ_BYTES: u64 = 2 * 1024 * 1024;

/// How far into a file a NUL byte is looked for.
const SNIFF_BYTES: usize = 8 * 1024;

/// How many entries a recursive copy will walk.
///
/// A folder copy is the one op here whose cost is not bounded by the path it names.
const MAX_COPY_ENTRIES: usize = 20_000;

// ── the work itself ─────────────────────────────────────────────────

/// List `rel_path`, and the directories below it when `depth` is more than one.
///
/// Breadth first, so a shallow row is never waiting behind a deep one, and stopping at the reply
/// ceiling. The first listing is always the directory that was asked for.
pub fn listing(root: &Path, rel_path: &str, depth: u8) -> Result<Vec<DirListing>, FileError> {
    let depth = depth.clamp(1, MAX_DEPTH);
    let mut listings = Vec::new();
    let mut carried = 0usize;
    // The directory asked for is always listed; anything below it is only reached by descending.
    let mut level = vec![rel_path.to_string()];

    for reached in 0..depth {
        let mut next = Vec::new();
        for parent in level {
            let listed = match one_level(root, &parent) {
                Ok(listed) => listed,
                // The directory that was asked for is the request: if it fails, the request failed.
                // A directory only reached by descending is a convenience, and one that has gone or
                // cannot be read is skipped rather than costing the reply every row above it.
                Err(error) if reached == 0 => return Err(error),
                Err(error) => {
                    tracing::debug!("not descending into {parent:?}: {error}");
                    continue;
                }
            };

            for entry in &listed.entries {
                // A deeper level is a convenience, so it obeys the ignore set; the level that was
                // asked for was answered above, whatever it is called.
                if reached + 1 < depth
                    && entry.kind == EntryKind::Dir
                    && !WALK_SKIP.contains(&entry.name.as_str())
                {
                    next.push(entry.rel_path.clone());
                }
            }

            carried += listed.entries.len();
            listings.push(listed);
            if carried >= MAX_REPLY_ENTRIES {
                return Ok(listings);
            }
        }
        level = next;
        if level.is_empty() {
            break;
        }
    }

    Ok(listings)
}

/// One directory, with every entry classified.
fn one_level(root: &Path, rel_path: &str) -> Result<DirListing, FileError> {
    let dir = path::resolve(root, rel_path)?;
    if !dir.is_dir() {
        return Err(FileError::WrongKind);
    }

    let reader = fs::read_dir(&dir).map_err(from_io)?;
    let mut entries = Vec::new();
    let mut truncated = false;

    for found in reader {
        let found = match found {
            Ok(found) => found,
            // One unreadable entry is not a reason to lose the directory around it.
            Err(error) => {
                tracing::debug!("skipping an entry in {}: {error}", dir.display());
                continue;
            }
        };

        let name = found.file_name().to_string_lossy().into_owned();
        if LIST_HIDE.contains(&name.as_str()) {
            continue;
        }
        if entries.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let child_rel = path::child(rel_path, &name);
        entries.push(classify(root, &found, name, child_rel));
    }

    // Directories first, then names compared without case, with the raw name breaking a tie so the
    // order is total and stable — two windows on one project must not disagree, and neither
    // re-sorts what the host sent.
    entries.sort_by(|a, b| {
        let group = |kind: EntryKind| u8::from(kind != EntryKind::Dir);
        group(a.kind)
            .cmp(&group(b.kind))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });

    Ok(DirListing {
        rel_path: rel_path.to_string(),
        entries,
        truncated,
    })
}

/// What one directory entry is.
///
/// `file_type` comes from the directory read itself, so the common case costs no extra syscall.
/// Only a symlink is looked at again: it is classified by what it points at, and only when that is
/// inside the root.
fn classify(root: &Path, found: &fs::DirEntry, name: String, rel_path: String) -> DirEntry {
    let file_type = found.file_type().ok();
    let symlink = file_type.is_some_and(|t| t.is_symlink());

    let (kind, size) = if symlink {
        // Resolving through `path` is what refuses a link out of the root, so a link the host will
        // not follow is drawn as something it will not open either.
        match path::resolve(root, &rel_path)
            .and_then(|target| fs::metadata(target).map_err(from_io))
        {
            Ok(target) if target.is_dir() => (EntryKind::Dir, None),
            Ok(target) if target.is_file() => (EntryKind::File, Some(target.len())),
            Ok(_) | Err(_) => (EntryKind::Other, None),
        }
    } else {
        match file_type {
            Some(t) if t.is_dir() => (EntryKind::Dir, None),
            Some(t) if t.is_file() => (EntryKind::File, found.metadata().ok().map(|m| m.len())),
            // A socket, a device or a pipe. Drawn, never opened.
            _ => (EntryKind::Other, None),
        }
    };

    DirEntry {
        name,
        rel_path,
        kind,
        size,
        symlink,
    }
}

/// Read a file's prefix. `max_bytes` narrows [`MAX_READ_BYTES`] and never widens it.
pub fn contents(
    root: &Path,
    rel_path: &str,
    max_bytes: Option<u64>,
) -> Result<FileContents, FileError> {
    let file = path::resolve(root, rel_path)?;

    // Stat before opening, and require a regular file. A read on a directory fails with `EISDIR`
    // on Linux but can *succeed* on macOS, and a FIFO or a device would block this thread forever
    // — which is the sharpest failure in the whole family, and why `EntryKind::Other` exists.
    let stat = fs::metadata(&file).map_err(from_io)?;
    if !stat.is_file() {
        return Err(FileError::WrongKind);
    }

    let limit = max_bytes.unwrap_or(MAX_READ_BYTES).min(MAX_READ_BYTES);
    let handle = fs::File::open(&file).map_err(from_io)?;
    let mut bytes = Vec::new();
    // One byte past the limit, so truncation is derived from what was actually read rather than
    // from the stat above: a file that grew in between still reports itself honestly.
    handle
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(from_io)?;

    let truncated = bytes.len() as u64 > limit;
    if truncated {
        bytes.truncate(limit as usize);
    }

    let is_binary = looks_binary(&bytes);

    // Taken after the read, and withheld when the read was cut short. A file being written while
    // it is read cannot be made consistent here; the version guard on the save is what covers it.
    let version = if truncated {
        None
    } else {
        fs::metadata(&file).ok().map(version_of)
    };
    let len = fs::metadata(&file).map(|m| m.len()).unwrap_or(stat.len());

    Ok(FileContents {
        bytes,
        len,
        truncated,
        is_binary,
        version,
    })
}

/// Write a file whole, atomically, refusing a stale version.
///
/// `expected` present is an overwrite that must land on exactly the file that was read.
/// `expected` absent is a creation, and is refused if anything is already there — the only safe
/// meaning it can have, because the alternative reading is a forced overwrite the contract would be
/// handing out for free.
pub fn save(
    root: &Path,
    rel_path: &str,
    bytes: &[u8],
    expected: Option<FileVersion>,
) -> Result<FileVersion, FileError> {
    // First, so containment is settled before a single byte is written.
    let file = path::resolve_for_write(root, rel_path)?;
    let current = match fs::metadata(&file) {
        Ok(stat) if stat.is_file() => Some(stat),
        Ok(_) => return Err(FileError::WrongKind),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(from_io(error)),
    };

    let mode = match (&expected, &current) {
        (Some(expected), Some(stat)) => {
            if version_of(stat.clone()) != *expected {
                return Err(FileError::Conflict);
            }
            Some(stat.permissions())
        }
        // A save is not a resurrection: the tab is stale, and saying so is more use than silently
        // recreating a file somebody deleted.
        (Some(_), None) => return Err(FileError::Missing),
        (None, Some(_)) => return Err(FileError::Conflict),
        (None, None) => None,
    };

    crate::atomic::write_atomic_with(&file, bytes, mode).map_err(from_io)?;
    fs::metadata(&file).map(version_of).map_err(from_io)
}

/// Create, move, copy or remove one path.
///
/// Containment is settled before anything on disk is touched, and every destination must be free:
/// the alternative reading of a taken destination is a forced overwrite, which is what [`save`]
/// refuses for the same reason.
///
/// `to` is the destination, and it belongs to `Move` and `Copy` alone. Present where it is not
/// wanted, or absent where it is, it is a refusal rather than something quietly dropped — a `to`
/// the host ignores is a wiring mistake the interface cannot see.
pub fn edit(root: &Path, rel_path: &str, to: Option<&str>, op: PathOp) -> Result<(), FileError> {
    let wants_to = matches!(op, PathOp::Move | PathOp::Copy);
    if wants_to != to.is_some() {
        return Err(FileError::Refused(format!(
            "{op:?} carries the wrong destination"
        )));
    }

    match op {
        PathOp::Create { dir } => {
            let target = path::resolve_for_write(root, rel_path)?;
            if target.exists() {
                return Err(FileError::Conflict);
            }
            if dir {
                fs::create_dir(&target).map_err(from_io)
            } else {
                crate::atomic::write_atomic_with(&target, b"", None).map_err(from_io)
            }
        }
        PathOp::Move | PathOp::Copy => {
            // A move takes its source away, so the project's own root is refused by name; a copy
            // only reads it, and copying a whole project somewhere inside itself is refused below
            // as any other self-containment is.
            let source = if op == PathOp::Move {
                path::resolve_inside(root, rel_path)?
            } else {
                path::resolve(root, rel_path)?
            };
            let target = path::resolve_for_write(root, to.unwrap_or_default())?;
            if target.exists() {
                return Err(FileError::Conflict);
            }
            // Whole components, on the canonical source and a destination whose parent is
            // canonical: a folder moved into its own child would either lose the tree or copy
            // forever, and the interface refuses the gesture too rather than raising a dialog for
            // it.
            if target.starts_with(&source) {
                return Err(FileError::Refused(
                    "a folder cannot go inside itself".to_string(),
                ));
            }

            if op == PathOp::Move {
                fs::rename(&source, &target).map_err(from_io)
            } else if fs::metadata(&source).map_err(from_io)?.is_dir() {
                let mut budget = MAX_COPY_ENTRIES;
                copy_tree(&source, &target, &mut budget)
            } else {
                fs::copy(&source, &target).map(|_| ()).map_err(from_io)
            }
        }
        PathOp::Trash => {
            let target = path::resolve_inside(root, rel_path)?;
            // The platform's own service answers this, so its refusal is not one of ours: a
            // headless session has no trash at all, and saying so is better than deleting instead.
            trash::delete(&target).map_err(|error| FileError::Failed(error.to_string()))
        }
        PathOp::Delete => {
            let target = path::resolve_inside(root, rel_path)?;
            if fs::metadata(&target).map_err(from_io)?.is_dir() {
                fs::remove_dir_all(&target).map_err(from_io)
            } else {
                fs::remove_file(&target).map_err(from_io)
            }
        }
    }
}

/// Copy one folder with everything under it, spending `budget` as it goes.
///
// ponytail: the ceiling is entries walked, and a copy that hits it stops where it stopped —
// what is already on disk is left there, named by the error rather than rolled back. Upgrade
// path: copy into a sibling temporary folder and rename it into place at the end, which is what
// `atomic::write_atomic_with` does for one file.
fn copy_tree(source: &Path, target: &Path, budget: &mut usize) -> Result<(), FileError> {
    fs::create_dir(target).map_err(from_io)?;
    for found in fs::read_dir(source).map_err(from_io)? {
        let found = found.map_err(from_io)?;
        if *budget == 0 {
            return Err(FileError::Failed(format!(
                "a folder copy stops at {MAX_COPY_ENTRIES} entries"
            )));
        }
        *budget -= 1;

        let child = target.join(found.file_name());
        if found.file_type().map_err(from_io)?.is_dir() {
            copy_tree(&found.path(), &child, budget)?;
        } else {
            // A symlink is copied as what it points at, which is what `fs::copy` does and what a
            // copy inside one project should mean; a link to a folder fails, and says so.
            fs::copy(found.path(), &child).map_err(from_io)?;
        }
    }
    Ok(())
}

/// Whether the host will treat these bytes as text.
///
/// A NUL in the first few kilobytes, which is what git does. Not a UTF-8 check: a Latin-1 file, and
/// a UTF-8 one cut mid-sequence, are both still text the user wants to see. The read reports it and
/// the diff obeys it, so a file drawn in a viewer rather than an editor is never diffed either.
fn looks_binary(bytes: &[u8]) -> bool {
    let sniff = SNIFF_BYTES.min(bytes.len());
    bytes[..sniff].contains(&0)
}

/// What a stat says about a file, as the write guard compares it.
fn version_of(stat: fs::Metadata) -> FileVersion {
    FileVersion {
        len: stat.len(),
        modified: stat
            .modified()
            .ok()
            .map(chrono::DateTime::<chrono::Utc>::from),
    }
}

/// The operating system's refusal as the contract's.
fn from_io(error: std::io::Error) -> FileError {
    match error.kind() {
        std::io::ErrorKind::NotFound => FileError::Missing,
        std::io::ErrorKind::PermissionDenied => FileError::Denied(error.to_string()),
        _ => FileError::Failed(error.to_string()),
    }
}

// ── the worker ──────────────────────────────────────────────────────

/// What one file-family request is, once the coordinator has resolved which project it is for.
pub enum Request {
    Tree {
        rel_path: String,
        depth: u8,
    },
    Read {
        rel_path: String,
        max_bytes: Option<u64>,
    },
    Write {
        rel_path: String,
        bytes: Vec<u8>,
        expected: Option<FileVersion>,
    },
    Diff {
        rel_path: String,
        base: DiffBase,
    },
    Edit {
        rel_path: String,
        to: Option<String>,
        op: PathOp,
    },
}

/// One request, addressed.
///
/// It carries the root rather than a way to look one up, so the worker never needs a lock on
/// anything the coordinator owns.
pub struct Job {
    pub project_id: ProjectId,
    /// The record's path, taken from memory on the coordinator's thread. Resolving it against a
    /// `rel_path` — and every syscall that takes — happens on the worker.
    pub root: PathBuf,
    pub request: Request,
    /// The window that asked. A [`Mailbox`] already knows who it is talking to, which is the same
    /// device a pane's reader and its reaper use.
    pub reply_to: Mailbox,
}

/// The thread that answers the file family.
///
/// **One thread, not a pool.** With a single worker and a first-in-first-out queue, replies to one
/// window arrive in the order they were asked for, which is what makes "replace the rows under this
/// path" safe in the interface. A pool reorders, so two clicks on one folder could leave the older
/// answer on screen — and fixing that would cost a sequence number on the wire. Every request is
/// bounded, and a slow one delaying the next is what the user asked for by clicking twice.
pub struct Files {
    jobs: flume::Sender<Job>,
}

impl Files {
    /// Start the worker. It ends when the coordinator that holds this drops it.
    pub fn start() -> Self {
        let (jobs, queue) = flume::unbounded::<Job>();
        thread::Builder::new()
            .name("ubiq-files".to_string())
            .spawn(move || {
                while let Ok(job) = queue.recv() {
                    let message = answer(&job);
                    // A window that has gone is not an error: nothing is left to draw the answer.
                    job.reply_to.send(message);
                }
            })
            .expect("the files thread");
        Self { jobs }
    }

    /// Queue a request. Never blocks — the queue is unbounded, on the bus's own rule.
    pub fn submit(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            tracing::error!("the files thread has gone; a request was dropped");
        }
    }
}

/// Do one job and say what the window is told.
fn answer(job: &Job) -> Message {
    let project_id = job.project_id;
    match &job.request {
        Request::Tree { rel_path, depth } => match listing(&job.root, rel_path, *depth) {
            Ok(listings) => Message::ProjectTreeListing {
                project_id,
                rel_path: rel_path.clone(),
                listings,
            },
            Err(error) => file_error(project_id, rel_path, error),
        },
        Request::Read {
            rel_path,
            max_bytes,
        } => match contents(&job.root, rel_path, *max_bytes) {
            Ok(contents) => Message::ProjectFileContents {
                project_id,
                rel_path: rel_path.clone(),
                contents,
            },
            Err(error) => file_error(project_id, rel_path, error),
        },
        Request::Write {
            rel_path,
            bytes,
            expected,
        } => match save(&job.root, rel_path, bytes, *expected) {
            Ok(version) => Message::ProjectFileWritten {
                project_id,
                rel_path: rel_path.clone(),
                version,
            },
            Err(error) => file_error(project_id, rel_path, error),
        },
        Request::Diff { rel_path, base } => match diff::diff(&job.root, rel_path, *base) {
            Ok(diff) => Message::ProjectFileDiffed {
                project_id,
                rel_path: rel_path.clone(),
                diff,
            },
            Err(error) => file_error(project_id, rel_path, error),
        },
        Request::Edit { rel_path, to, op } => match edit(&job.root, rel_path, to.as_deref(), *op) {
            Ok(()) => Message::ProjectPathEdited {
                project_id,
                rel_path: rel_path.clone(),
                to: to.clone(),
                op: *op,
            },
            Err(error) => file_error(project_id, rel_path, error),
        },
    }
}

/// One path's failure, addressed so the interface can mark the row or the tab it belongs to.
pub fn file_error(project_id: ProjectId, rel_path: &str, error: FileError) -> Message {
    if let FileError::Refused(reason) = &error {
        // A refusal is a wiring mistake rather than something the user did: the interface only ever
        // holds paths the host handed it.
        tracing::warn!("refused {rel_path:?} in project {project_id}: {reason}");
    }
    Message::ProjectFileError {
        project_id,
        rel_path: rel_path.to_string(),
        error,
    }
}
