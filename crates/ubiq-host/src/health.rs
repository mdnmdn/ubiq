//! What is actually at a project's path.
//!
//! The directory is the part of a project the host does not control, and every interesting failure
//! is a variation on it going away. One `symlink_metadata` per record makes the boot probe cheap
//! enough to be unconditional.

use std::path::Path;

use ubiq_proto::projects::ProjectHealth;

/// Look at the path once and say what is there.
///
/// `symlink_metadata` rather than `metadata`, so a symlink is seen as itself first: a link that
/// leads nowhere is `NotADirectory`, which is a fact about the record, and not `Missing`, which
/// would invite a Locate that cannot help.
pub fn probe(path: &Path) -> ProjectHealth {
    let link = match std::fs::symlink_metadata(path) {
        Ok(link) => link,
        Err(error) => {
            return match error.kind() {
                std::io::ErrorKind::NotFound => ProjectHealth::Missing,
                _ => ProjectHealth::Unreadable(reason(&error)),
            };
        }
    };

    if !link.file_type().is_symlink() {
        return if link.is_dir() {
            ProjectHealth::Ok
        } else {
            ProjectHealth::NotADirectory
        };
    }

    // A symlink costs a second look, at what it points to.
    match std::fs::metadata(path) {
        Ok(target) if target.is_dir() => ProjectHealth::Ok,
        // A broken link, or one pointing at a file: something is there, and it is not a project.
        Ok(_) | Err(_) => ProjectHealth::NotADirectory,
    }
}

/// The operating system's own words, which are more use than anything this could invent.
fn reason(error: &std::io::Error) -> String {
    error.to_string()
}
