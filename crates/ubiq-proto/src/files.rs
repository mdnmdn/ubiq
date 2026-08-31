//! What a project's files are on the wire: one level of a tree, one file's bytes, and the failures
//! a single path can have.
//!
//! **The interface holds project-relative paths only.** A `rel_path` is forward-slashed, has no
//! leading slash and no `..`, and is empty for the project's root. It is resolved against the
//! record's root by the host, which is the file-level form of the rule that the interface never
//! assumes the pseudo-terminal is local — and the seam a remote drone slots into without the
//! interface noticing, because a project id and a relative path do not say which machine answered.
//!
//! Contents cross as bytes. The host does not decode a file any more than it decodes terminal
//! output: a truncated read can cut a multi-byte sequence, a binary file has no `String` at all,
//! and which encoding to draw is the interface's decision.
//!
//! This is a module of its own rather than part of [`crate::projects`], which owns the split
//! between the durable record and the derived snapshot. A directory entry is neither: it is a
//! per-request payload with no durability story and no relation to a `ProjectRecord`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What one entry in a listing is.
///
/// A symlink is classified by what it points at, and only when that is inside the project's root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    /// A directory. A further [`crate::messages::Message::ProjectTree`] lists it.
    Dir,
    /// A regular file. [`crate::messages::Message::ReadProjectFile`] reads it.
    File,
    /// Something the host will not follow or open: a symlink leading out of the root or nowhere, a
    /// socket, a device, a pipe. It is still drawn, because a row the interface never sees is a
    /// tree that lies, and it is refused when it is asked for.
    Other,
}

/// One entry, as the explorer draws it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    /// The leaf name, which is what the row shows.
    pub name: String,
    /// Project-relative, built from the parent's `rel_path` and `name` — never from a path on
    /// disk, so no absolute path can be formed into a message by mistake.
    pub rel_path: String,
    pub kind: EntryKind,
    /// Size in bytes, absent for anything that is not a regular file. It is what lets the
    /// interface warn before opening something large, which it cannot learn any other way.
    pub size: Option<u64>,
    /// Whether the entry is a symlink, whatever it points at. The directory read already says so,
    /// and it is the difference between a folder and a link to one.
    pub symlink: bool,
}

/// One directory, listed.
///
/// A listing is always exactly one level, so the reply's shape does not depend on the depth that
/// was asked for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirListing {
    /// The directory this lists, project-relative. Empty is the project's root.
    pub rel_path: String,
    /// Directories first, then names compared without case. The host sorts, so two windows agree
    /// and neither re-sorts.
    pub entries: Vec<DirEntry>,
    /// Whether the entry ceiling cut the listing short. The interface says so on the row rather
    /// than drawing a folder as smaller than it is.
    pub truncated: bool,
}

/// What a file was when it was read: enough to refuse a write that would land on somebody else's
/// change, and free from the metadata the read already takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileVersion {
    pub len: u64,
    /// Last modification as the filesystem reported it. Absent on a filesystem that has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<DateTime<Utc>>,
}

/// A file's bytes and what the host knows about them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileContents {
    /// Opaque, like terminal bytes: a prefix from offset zero, and the interface decides what it
    /// can draw.
    pub bytes: Vec<u8>,
    /// The file's whole length, which is larger than `bytes` when the read was cut short.
    pub len: u64,
    /// Whether the ceiling stopped the read. It never means a middle is missing — only that this
    /// is a prefix and the file is longer.
    pub truncated: bool,
    /// A NUL byte in the first few kilobytes. The interface shows a viewer rather than an editor.
    pub is_binary: bool,
    /// What to hand back with a save. **Absent when `truncated`**, which is what makes a truncated
    /// buffer unsavable mechanically rather than by the interface remembering not to: a write
    /// naming no version is refused on a file that already exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<FileVersion>,
}

/// What went wrong for one path.
///
/// An enum rather than a sentence, because each of these is a different thing for the interface to
/// do: `Missing` means drop the row, `Conflict` means offer to overwrite, `Denied` means mark it,
/// and `Refused` means a wiring mistake worth a log line. A string would force the interface to
/// match on prose.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "reason")]
pub enum FileError {
    /// The path escaped the root after normalisation, named a link the host will not follow, or
    /// named a project that is not in the catalogue. The interface cannot fix it.
    Refused(String),
    /// Nothing is there any more. The row and the tab are stale.
    Missing,
    /// Not the kind of thing the request needs: a directory read as a file, a file listed as a
    /// directory, or a socket, a device or a pipe.
    WrongKind,
    /// The operating system refused, in its own words.
    Denied(String),
    /// The file changed since it was read, so the write was not made and nothing was lost.
    Conflict,
    /// Anything else the operating system said.
    Failed(String),
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileError::Refused(reason) => write!(f, "refused: {reason}"),
            FileError::Missing => write!(f, "it is not there"),
            FileError::WrongKind => write!(f, "not the kind of thing this reads"),
            FileError::Denied(reason) => write!(f, "{reason}"),
            FileError::Conflict => write!(f, "it changed since it was read"),
            FileError::Failed(reason) => write!(f, "{reason}"),
        }
    }
}
