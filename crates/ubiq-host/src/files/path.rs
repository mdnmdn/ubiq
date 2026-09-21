//! Resolving a project-relative path against a project's root, or refusing it.
//!
//! This is the security boundary of the file family, and it is a module of its own so that it can
//! be read and tested with nothing else in scope. Every path that reaches the filesystem goes
//! through here first.
//!
//! **Two checks, and neither is sufficient alone.** The components are refused textually, which
//! stops a path from ever *naming* somewhere outside the root; then every symlink is resolved and
//! the result must still be inside the root, which stops a path from *leading* outside one. A
//! textual check alone is defeated by a symlink; a canonicalising check alone is defeated by a root
//! that does not exist yet, and is easier to get subtly wrong.

use std::path::{Component, Path, PathBuf};

use ubiq_proto::files::FileError;

/// The most components a project-relative path may have.
///
/// Nothing a user clicks through is anywhere near this deep, and a bound is what stops a crafted
/// string from turning one request into thousands of syscalls.
const MAX_COMPONENTS: usize = 64;

/// Resolve a project-relative path that must already exist, or refuse it.
///
/// The path is resolved against `root`, every symlink in it is followed, and the result has to be
/// inside `root` when it lands.
pub fn resolve(root: &Path, rel_path: &str) -> Result<PathBuf, FileError> {
    let parts = components(rel_path)?;
    let root = canonical(root)?;
    let target = canonical(&root.join(parts))?;
    contain(&root, target)
}

/// The same for a path being written.
///
/// The **parent** must exist and be contained, because a write creates the leaf and cannot
/// canonicalise it; the leaf must not be a symlink, since a write through a link is a write
/// wherever the link points; and no folder is ever created to make the path valid. A write that
/// names a version says the file is already on disk, so a folder of its own missing is a fact
/// about the world the interface's tab no longer matches — inventing one would hide that.
/// [`resolve_for_create`] is the one door that makes folders, and only a creation takes it.
pub fn resolve_for_write(root: &Path, rel_path: &str) -> Result<PathBuf, FileError> {
    let parts = components(rel_path)?;
    let Some(leaf) = parts.file_name().map(|name| name.to_owned()) else {
        return Err(FileError::Refused(
            "a write needs a file to write to".to_string(),
        ));
    };

    let root = canonical(root)?;
    let parent = match parts.parent() {
        Some(parent) => contain(&root, canonical(&root.join(parent))?)?,
        None => root.clone(),
    };

    let target = parent.join(leaf);
    // The leaf itself is only checked for being a link, because it does not have to exist. A link
    // is refused whatever it points at: following one would put the containment check on the wrong
    // path, and replacing one silently would be worse still.
    if let Ok(link) = std::fs::symlink_metadata(&target)
        && link.file_type().is_symlink()
    {
        return Err(FileError::Refused(
            "that name is a symlink, and a write is never followed through one".to_string(),
        ));
    }
    Ok(target)
}

/// The same as [`resolve_for_write`], bringing the folders the path names into existence.
///
/// **Only a creation takes this door** — a write naming no version, which is the interface saying
/// this buffer has never been on disk. That is the whole difference: a save-as asks the user for a
/// project-relative *path*, so the folders in it are part of the file being created, and refusing
/// the write because one of them is not there yet reports a file that never existed as one that
/// went away. An overwrite and a versioned save both name something that is supposed to exist
/// already, and they keep refusing (see [`resolve_for_write`]) — the protection this relaxes is
/// only ever relaxed where there is nothing on disk to protect.
///
/// The boundary is unchanged: every level is created **one at a time under an already-contained
/// canonical ancestor**, and each one is canonicalised and contained again before the next is
/// joined onto it, so an existing component that is a symlink out of the root is refused exactly as
/// it would be by [`resolve_for_write`]. Nothing is created outside the root, and the root itself
/// is never created.
pub fn resolve_for_create(root: &Path, rel_path: &str) -> Result<PathBuf, FileError> {
    let parts = components(rel_path)?;
    let Some(parent_parts) = parts.parent() else {
        return resolve_for_write(root, rel_path);
    };

    let root = canonical(root)?;
    let mut dir = root.clone();
    for component in parent_parts.components() {
        let next = dir.join(component);
        match std::fs::create_dir(&next) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(super::from_io(error)),
        }
        dir = contain(&root, canonical(&next)?)?;
    }

    resolve_for_write(&root, rel_path)
}

/// The same as [`resolve`], refusing the project's own root.
///
/// A delete or a move needs something *inside* the project. An empty `rel_path` resolves to the
/// root itself, and `remove_dir_all` on it would take the project with it — so the one path the
/// interface can name that must never be an argument is refused by name.
pub fn resolve_inside(root: &Path, rel_path: &str) -> Result<PathBuf, FileError> {
    if rel_path.trim().is_empty() {
        return Err(FileError::Refused(
            "that is the project's own folder".to_string(),
        ));
    }
    resolve(root, rel_path)
}

/// A listed child's project-relative path, from its parent's and its own name.
///
/// The one way a `rel_path` is ever constructed, so nothing on disk can leak into a message.
pub fn child(parent_rel: &str, name: &str) -> String {
    if parent_rel.is_empty() {
        name.to_string()
    } else {
        format!("{parent_rel}/{name}")
    }
}

/// The path's own components, refused if any of them is anything but a plain name.
///
/// `Path::components` rather than a split on `'/'`, because that is what makes `/etc/passwd` a
/// `RootDir` and `C:\…` a `Prefix` under each platform's own rules, rather than something that
/// silently reads as root-relative.
///
/// **`..` is refused, never popped.** The interface only ever holds paths the host handed it, so it
/// never needs one; popping would turn a crafted string into a probe of the root's parent as an
/// arithmetic accident.
fn components(rel_path: &str) -> Result<PathBuf, FileError> {
    if rel_path.contains('\0') {
        return Err(FileError::Refused("a path holds a NUL byte".to_string()));
    }

    let mut parts = PathBuf::new();
    let mut depth = 0;
    for component in Path::new(rel_path).components() {
        match component {
            Component::Normal(name) => {
                parts.push(name);
                depth += 1;
            }
            // `./` says nothing, so it costs nothing to drop.
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(FileError::Refused("a path leaves the project".to_string()));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(FileError::Refused(
                    "a path is absolute, and the interface holds project-relative paths only"
                        .to_string(),
                ));
            }
        }
    }

    if depth > MAX_COMPONENTS {
        return Err(FileError::Refused("a path is absurdly deep".to_string()));
    }
    Ok(parts)
}

/// Canonicalise, turning the operating system's refusal into the contract's.
///
/// The **root** goes through this too, on every request. Omitting it is the classic form of this
/// bug: a project under `/tmp` canonicalises to `/private/tmp/…` on macOS, so every legitimate
/// child would fail containment against the uncanonicalised root. It also catches a root that has
/// itself become a symlink since the record was written.
fn canonical(path: &Path) -> Result<PathBuf, FileError> {
    std::fs::canonicalize(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => FileError::Missing,
        std::io::ErrorKind::PermissionDenied => FileError::Denied(error.to_string()),
        _ => FileError::Failed(error.to_string()),
    })
}

/// Whether a canonical path is inside a canonical root.
///
/// `Path::starts_with` compares whole components, so `/p/rootsibling` is not inside `/p/root` —
/// which a string prefix test would get wrong.
fn contain(root: &Path, target: PathBuf) -> Result<PathBuf, FileError> {
    if target.starts_with(root) {
        Ok(target)
    } else {
        Err(FileError::Refused(
            "that path leads outside the project".to_string(),
        ))
    }
}
