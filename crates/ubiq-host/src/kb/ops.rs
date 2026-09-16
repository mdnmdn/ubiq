//! Mutating one knowledge-base source: writing a document whole, creating or renaming an entry,
//! deleting one, and answering where one sits on disk.
//!
//! Free functions rather than methods on [`crate::kb::Kb`], on purpose: the coordinator is one
//! caller, reached through [`crate::messages`]'s bus, and it will not be the only one — an MCP
//! server reaches the same knowledge base without a window in the room, and it has no bus to go
//! through. Both callers resolve a source the same way `Kb::find` and `Kb::base_path` already do,
//! and then land here.
//!
//! **Writability is a real check, not a courtesy.** Every mutating function below opens with
//! [`require_writable`], which refuses a source [`ubiq_proto::kb::KbSource::is_writable`] says no
//! to. A caller that skipped it would let a stale or forged message write into a folder the user
//! marked read-only — the check earns its keep precisely because nothing forces a caller to make
//! it twice.
//!
//! **The filter never gates a write.** [`ubiq_proto::kb::KbSource::admits`] decides what a
//! source's *tree shows*; consulting it here would mean a document the user just typed a name for
//! could be refused for the crime of not matching the glob that is about to hide it — a trap, not
//! a protection. Nothing in this module reads `filter`.
//!
//! Containment is [`crate::files::path`]'s, the same boundary the file family resolves every
//! `rel_path` through — a second implementation here would be a second place for the two to drift
//! apart.

use std::path::{Path, PathBuf};

use ubiq_proto::files::{EntryKind, FileError};
use ubiq_proto::kb::KbSource;

use crate::files::path;

/// Refuse a mutating call against a source Ubiq may not write to.
fn require_writable(source: &KbSource) -> Result<(), FileError> {
    if source.is_writable() {
        Ok(())
    } else {
        Err(FileError::Refused(
            "this knowledge-base source is read-only".to_string(),
        ))
    }
}

/// `rel_path`'s own parent, on the wire's own convention that a `rel_path` is forward-slashed and
/// never has a leading separator. Empty is the source's own root, the same meaning
/// [`Message::KbTree`]'s empty `rel_path` carries.
///
/// `pub(crate)` rather than private: the coordinator names the same parent again, in
/// [`Message::KbChanged`], once an op here has already succeeded.
pub(crate) fn parent_of(rel_path: &str) -> String {
    match rel_path.rsplit_once('/') {
        Some((parent, _)) => parent.to_string(),
        None => String::new(),
    }
}

/// Write a document's whole contents, atomically. Refused for a source that is not writable.
///
/// `path::resolve_for_write` is what settles containment — the same resolver
/// [`crate::files::save`] uses — so the immediate parent has to exist and be inside the source
/// already; [`crate::atomic::write_atomic`] then creates nothing beyond confirming it, and the
/// rename-over-temp-file it does is what keeps a crash from leaving half a document.
pub fn write_file(
    base: &Path,
    source: &KbSource,
    rel_path: &str,
    contents: &str,
) -> Result<(), FileError> {
    require_writable(source)?;
    let target = path::resolve_for_write(base, rel_path)?;
    crate::atomic::write_atomic(&target, contents.as_bytes()).map_err(from_io)
}

/// Create an empty file or a folder at `rel_path`. Refused for a source that is not writable, and
/// for a path something already occupies — the same "no forced overwrite" rule
/// [`crate::files::save`] and [`crate::files::edit`] both keep.
pub fn create(
    base: &Path,
    source: &KbSource,
    rel_path: &str,
    kind: EntryKind,
) -> Result<(), FileError> {
    require_writable(source)?;
    let target = path::resolve_for_write(base, rel_path)?;
    if target.exists() {
        return Err(FileError::Conflict);
    }
    match kind {
        EntryKind::Dir => std::fs::create_dir(&target).map_err(from_io),
        EntryKind::File => crate::atomic::write_atomic(&target, b"").map_err(from_io),
        // Nothing the interface ever asks to create — a symlink, a socket, a device — belongs
        // here; refusing it is cheaper than a syscall that would only fail its own way.
        EntryKind::Other => Err(FileError::Refused(
            "that is not a kind of entry this can create".to_string(),
        )),
    }
}

/// Rename one entry in place. Refused for a source that is not writable, for a `new_name` that is
/// a path rather than a name, and for a `new_name` that would land on something already there.
///
/// Returns the entry's new `rel_path`, since the caller — the coordinator, answering
/// [`ubiq_proto::messages::Message::RenameKbEntry`] — has no other way to say what to re-list.
pub fn rename(
    base: &Path,
    source: &KbSource,
    rel_path: &str,
    new_name: &str,
) -> Result<String, FileError> {
    require_writable(source)?;
    if new_name.is_empty() || new_name.contains(['/', '\\']) || new_name == "." || new_name == ".."
    {
        return Err(FileError::Refused(
            "a new name must be a name, never a path".to_string(),
        ));
    }

    let from = path::resolve_inside(base, rel_path)?;
    let new_rel = path::child(&parent_of(rel_path), new_name);
    let to = path::resolve_for_write(base, &new_rel)?;
    if to.exists() {
        return Err(FileError::Conflict);
    }

    std::fs::rename(&from, &to).map_err(from_io)?;
    Ok(new_rel)
}

/// Delete one entry. A folder goes with everything under it. Refused for a source that is not
/// writable.
pub fn delete(base: &Path, source: &KbSource, rel_path: &str) -> Result<(), FileError> {
    require_writable(source)?;
    let target = path::resolve_inside(base, rel_path)?;
    let stat = std::fs::metadata(&target).map_err(from_io)?;
    if stat.is_dir() {
        std::fs::remove_dir_all(&target).map_err(from_io)
    } else {
        std::fs::remove_file(&target).map_err(from_io)
    }
}

/// The absolute path of one entry, for the clipboard.
///
/// No writability check: this reads nothing and writes nothing, and a read-only source's path is
/// exactly as clipboard-worthy as a writable one's.
pub fn absolute(base: &Path, _source: &KbSource, rel_path: &str) -> Result<PathBuf, FileError> {
    path::resolve(base, rel_path)
}

/// Show one path in the platform's file manager, on the machine this process runs on.
///
/// A directory is opened as itself; a file is revealed selected inside its parent — the one
/// distinction every platform's own tool draws differently, so each branch says it its own way
/// rather than one command line trying to mean all three.
pub fn reveal(path: &Path) -> std::io::Result<()> {
    if cfg!(target_os = "macos") {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map(|_| ())
    } else if cfg!(target_os = "windows") {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path.as_os_str());
        std::process::Command::new("explorer")
            .arg(arg)
            .spawn()
            .map(|_| ())
    } else {
        // `xdg-open` has no notion of "select this one inside its folder" the way Finder and
        // Explorer do, so the containing directory is the most it can promise.
        let dir = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        std::process::Command::new("xdg-open")
            .arg(dir)
            .spawn()
            .map(|_| ())
    }
}

/// The operating system's refusal as the contract's, on the same mapping [`crate::files`] uses for
/// the file family.
fn from_io(error: std::io::Error) -> FileError {
    match error.kind() {
        std::io::ErrorKind::NotFound => FileError::Missing,
        std::io::ErrorKind::PermissionDenied => FileError::Denied(error.to_string()),
        _ => FileError::Failed(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::ids::KbSourceId;
    use ubiq_proto::kb::{KbAccess, KbOrigin};

    fn source(access: KbAccess) -> KbSource {
        KbSource {
            id: KbSourceId::generate(),
            name: "docs".to_string(),
            origin: KbOrigin::Folder {
                path: "/unused".to_string(),
            },
            filter: String::new(),
            access,
        }
    }

    #[test]
    fn a_write_against_a_read_only_source_is_refused() {
        let base = tempfile::TempDir::new().unwrap();
        let source = source(KbAccess::ReadOnly);
        let error = write_file(base.path(), &source, "notes.md", "hi").unwrap_err();
        assert!(matches!(error, FileError::Refused(_)));
        assert!(!base.path().join("notes.md").exists());
    }

    #[test]
    fn a_write_against_a_writable_folder_source_lands_on_disk() {
        let base = tempfile::TempDir::new().unwrap();
        let source = source(KbAccess::ReadWrite);
        write_file(base.path(), &source, "notes.md", "hi").unwrap();
        assert_eq!(
            std::fs::read_to_string(base.path().join("notes.md")).unwrap(),
            "hi"
        );
    }

    #[test]
    fn an_internal_source_is_writable_without_access_saying_so() {
        let base = tempfile::TempDir::new().unwrap();
        let mut source = source(KbAccess::ReadOnly);
        source.origin = KbOrigin::Internal;
        write_file(base.path(), &source, "notes.md", "hi").unwrap();
        assert!(base.path().join("notes.md").exists());
    }

    #[test]
    fn a_parent_dir_escape_is_refused() {
        let base = tempfile::TempDir::new().unwrap();
        let source = source(KbAccess::ReadWrite);
        let error = write_file(base.path(), &source, "../outside.md", "hi").unwrap_err();
        assert!(matches!(error, FileError::Refused(_)));
    }

    #[test]
    fn a_rename_with_a_separator_in_the_new_name_is_refused() {
        let base = tempfile::TempDir::new().unwrap();
        std::fs::write(base.path().join("a.md"), b"a").unwrap();
        let source = source(KbAccess::ReadWrite);
        let error = rename(base.path(), &source, "a.md", "sub/b.md").unwrap_err();
        assert!(matches!(error, FileError::Refused(_)));
        assert!(base.path().join("a.md").exists());
    }

    #[test]
    fn a_rename_onto_an_existing_name_is_refused() {
        let base = tempfile::TempDir::new().unwrap();
        std::fs::write(base.path().join("a.md"), b"a").unwrap();
        std::fs::write(base.path().join("b.md"), b"b").unwrap();
        let source = source(KbAccess::ReadWrite);
        let error = rename(base.path(), &source, "a.md", "b.md").unwrap_err();
        assert!(matches!(error, FileError::Conflict));
        assert_eq!(
            std::fs::read_to_string(base.path().join("b.md")).unwrap(),
            "b"
        );
    }

    #[test]
    fn delete_removes_a_folder_and_its_contents() {
        let base = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(base.path().join("sub")).unwrap();
        std::fs::write(base.path().join("sub/a.md"), b"a").unwrap();
        let source = source(KbAccess::ReadWrite);
        delete(base.path(), &source, "sub").unwrap();
        assert!(!base.path().join("sub").exists());
    }
}
