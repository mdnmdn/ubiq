//! Listing an absolute path on the host's own filesystem, with no project in scope.
//!
//! This is the host-browse family's worker logic. Unlike [`super::path`], there is no root to
//! resolve against and nothing to contain: browsing here is what happens *before* a project
//! exists, so it is the one place in the file family's neighbourhood that is allowed to see an
//! absolute path at all. `directories::BaseDirs` gives the default starting place when the caller
//! has none — the user's home directory, the same source `cli_shortcut.rs` and `config.rs`
//! already use for it.

use std::fs;
use std::path::PathBuf;

use ubiq_proto::files::{EntryKind, HostDirEntry, HostPathError};
use ubiq_proto::messages::Message;

/// One directory's ceiling.
///
/// Independent of the file family's own [`super::MAX_ENTRIES`]: a browse listing is always exactly
/// one level, so there is no reply-wide ceiling across several listings to share.
const MAX_ENTRIES: usize = 2_000;

/// One absolute directory, listed.
#[derive(Debug)]
pub struct Listing {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<HostDirEntry>,
    pub truncated: bool,
}

/// List `path`, or a sensible default when there is none.
///
/// The path is canonicalised before it is listed, so a request answered `Ok` always carries where
/// the host actually landed — a relative string, `.`, or a trailing symlink all resolve to the
/// same absolute answer a picker can walk from.
pub fn list(path: Option<&str>) -> Result<Listing, HostPathError> {
    let target = match path {
        Some(path) => PathBuf::from(path),
        None => default_path()?,
    };

    let canonical = fs::canonicalize(&target).map_err(from_io)?;
    if !canonical.is_dir() {
        return Err(HostPathError::NotADirectory);
    }

    let reader = fs::read_dir(&canonical).map_err(from_io)?;
    let mut entries = Vec::new();
    let mut truncated = false;

    for found in reader {
        let found = match found {
            Ok(found) => found,
            // One unreadable entry is not a reason to lose the directory around it, on the file
            // family's own rule.
            Err(error) => {
                tracing::debug!("skipping an entry in {}: {error}", canonical.display());
                continue;
            }
        };
        if entries.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        entries.push(classify(&found));
    }

    entries.sort_by(|a, b| super::dir_first_then_name(a.kind, &a.name, b.kind, &b.name));

    Ok(Listing {
        path: canonical.to_string_lossy().into_owned(),
        parent: canonical.parent().map(|p| p.to_string_lossy().into_owned()),
        entries,
        truncated,
    })
}

/// Do one browse job and say what the window is told.
pub fn answer(path: Option<&str>) -> Message {
    match list(path) {
        Ok(listing) => Message::HostDirListing {
            path: listing.path,
            parent: listing.parent,
            entries: listing.entries,
            truncated: listing.truncated,
        },
        Err(error) => Message::HostDirError {
            path: path.map(str::to_string),
            error,
        },
    }
}

/// The user's home directory: the host's own choice of a sensible starting place, made the same
/// way the CLI shortcut and the config root pick it.
fn default_path() -> Result<PathBuf, HostPathError> {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .ok_or_else(|| {
            HostPathError::Failed("could not find the user's home directory".to_string())
        })
}

/// What one entry is: its kind, whether it is a dotfile, and whether the host could open it.
///
/// Classified the way [`super::classify`] classifies a project entry — a symlink is drawn as what
/// it points at — except there is no root here for a link to escape, so nothing is refused for
/// leading outside one. A broken link or a loop is neither a directory nor a file, so it lands on
/// [`EntryKind::Other`] the same as a socket or a device would: drawn, never opened.
fn classify(found: &fs::DirEntry) -> HostDirEntry {
    let name = found.file_name().to_string_lossy().into_owned();
    let hidden = name.starts_with('.');

    let file_type = found.file_type().ok();
    let symlink = file_type.is_some_and(|t| t.is_symlink());
    let kind = if symlink {
        // `fs::metadata` follows the link; an error here covers a broken target and a loop alike,
        // since the operating system is what stops recursing on a loop, never this code.
        match fs::metadata(found.path()) {
            Ok(target) if target.is_dir() => EntryKind::Dir,
            Ok(target) if target.is_file() => EntryKind::File,
            _ => EntryKind::Other,
        }
    } else {
        match file_type {
            Some(t) if t.is_dir() => EntryKind::Dir,
            Some(t) if t.is_file() => EntryKind::File,
            _ => EntryKind::Other,
        }
    };

    // A best-effort hint, not a promise: open-and-drop is the cheapest true test of whether a
    // click would work, and it is what a picker needs to grey a row out before the click rather
    // than after.
    let readable = match kind {
        EntryKind::Dir => fs::read_dir(found.path()).is_ok(),
        EntryKind::File => fs::File::open(found.path()).is_ok(),
        EntryKind::Other => false,
    };

    HostDirEntry {
        name,
        kind,
        hidden,
        readable,
    }
}

/// The operating system's refusal as the contract's.
fn from_io(error: std::io::Error) -> HostPathError {
    match error.kind() {
        std::io::ErrorKind::NotFound => HostPathError::Missing,
        std::io::ErrorKind::PermissionDenied => HostPathError::Denied(error.to_string()),
        _ => HostPathError::Failed(error.to_string()),
    }
}
