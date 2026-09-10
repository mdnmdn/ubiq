//! The repositories *inside* the project: submodules the outer repository pins, and independent
//! trees it knows only as one untracked folder.
//!
//! The project's own repository is found by walking upward (`observe::open`). This is the other
//! direction: a bounded downward walk from the project's root that names every folder holding a
//! `.git`. It opens nothing — it answers paths, and `observe` decides what to do with them.

use std::fs;
use std::path::Path;

use ubiq_proto::files::WALK_SKIP;
use ubiq_proto::git::MAX_NESTED_REPOS;

/// How deep below the project a repository is still looked for.
///
/// A repository someone keeps eight folders down is a repository nobody browses the project for,
/// and an unbounded walk on a project holding a large unignored tree is the whole walk's cost paid
/// before a single badge is drawn. Past it the answer says `truncated` rather than pretending the
/// project has no more repositories.
pub const MAX_NESTED_DEPTH: usize = 8;

/// What the downward walk found.
pub struct Discovered {
    /// Working-tree roots, project-relative, forward-slashed, sorted. Never empty strings.
    pub roots: Vec<String>,
    /// A ceiling was reached: there may be repositories this walk did not look at.
    pub truncated: bool,
}

/// Every repository below `root`, bounded by [`MAX_NESTED_REPOS`] and [`MAX_NESTED_DEPTH`].
///
/// `root` itself is never a member: the project's own repository is the upward walk's answer.
pub fn discover(root: &Path) -> Discovered {
    let mut roots: Vec<String> = Vec::new();
    let mut truncated = false;
    let mut stack: Vec<(std::path::PathBuf, String, usize)> =
        vec![(root.to_path_buf(), String::new(), 0)];

    while let Some((dir, rel, depth)) = stack.pop() {
        if depth >= MAX_NESTED_DEPTH {
            truncated = true;
            continue;
        }
        let Ok(listing) = fs::read_dir(&dir) else {
            // An unreadable folder is not the project's failure: one denied directory must not
            // blank every badge in the project.
            continue;
        };
        for found in listing.flatten() {
            let name = found.file_name().to_string_lossy().to_string();
            // `WALK_SKIP` holds `.git` itself, along with `node_modules`, `target` and the rest of
            // what the file family never walks into. A repository under one of those is not a
            // repository the explorer draws.
            if WALK_SKIP.contains(&name.as_str()) {
                continue;
            }
            // `file_type` does not follow a symlink, which is how a link back up the tree cannot
            // make this walk loop.
            if !found.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let child_rel = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            let path = found.path();
            if holds_git(&path) {
                if roots.len() >= MAX_NESTED_REPOS {
                    roots.sort();
                    return Discovered {
                        roots,
                        truncated: true,
                    };
                }
                roots.push(child_rel);
                // Do not descend. A repository inside a repository inside the project is that
                // repository's business, not the project's — the nested one reports its own tree,
                // and its submodules are rows on its overview.
                continue;
            }
            stack.push((path, child_rel, depth + 1));
        }
    }

    roots.sort();
    Discovered { roots, truncated }
}

/// Whether `dir` is a repository's working-tree root.
///
/// A `.git` directory is the ordinary case; a `.git` **file** is a gitlink, which is how a
/// submodule and a linked worktree both appear. Either one means git owns this folder.
fn holds_git(dir: &Path) -> bool {
    dir.join(".git").exists()
}
