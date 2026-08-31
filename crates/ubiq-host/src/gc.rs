//! Collecting the directories of projects that are no longer in the catalogue.
//!
//! Forgetting a project drops the record first and its directory second, so a crash between the
//! two leaves a directory with no record. That is garbage, and this is where it is collected.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use ubiq_proto::ids::ProjectId;

/// Every `projects/<ulid>/` under `root` with no matching record.
///
/// A directory whose name is not a ULID is left alone and logged: never delete what you cannot
/// identify, because it was not this that put it there.
pub fn orphans(root: &Path, keep: &HashSet<ProjectId>) -> Vec<PathBuf> {
    let projects = root.join("projects");
    let Ok(entries) = std::fs::read_dir(&projects) else {
        return Vec::new();
    };

    let mut found = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };

        match ProjectId::from_str(name) {
            Ok(id) if !keep.contains(&id) => found.push(entry.path()),
            Ok(_) => {}
            Err(_) => {
                tracing::debug!(
                    "leaving {} alone: its name is not a project id",
                    entry.path().display()
                );
            }
        }
    }
    found.sort();
    found
}

/// Collect them. Answers how many went.
///
/// **Only ever called after a load that succeeded.** Running this against an empty catalogue that
/// came from a *corrupt* file would delete every project's view state, which is precisely the
/// thing preserving the file was meant to avoid.
pub fn collect(root: &Path, keep: &HashSet<ProjectId>) -> usize {
    let mut removed = 0;
    for path in orphans(root, keep) {
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                tracing::info!("collected {}, which no record names", path.display());
                removed += 1;
            }
            Err(error) => tracing::warn!("could not collect {}: {error}", path.display()),
        }
    }
    removed
}
