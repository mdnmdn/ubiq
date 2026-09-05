//! Lane layout for the commit graph: which column each commit draws in, and which columns a merge
//! commit's extra parents draw from.
//!
//! A pure function over one page of commits plus the lane state carried in from the page before
//! it — no repository access, no ancestor walk. See `_docs/wip/git-assumptions.md` §7 for why this
//! is hand-rolled rather than a dependency: nothing on crates.io exposes a lane-layout API over
//! bare ids and parents that also resumes across a cursor-paged walk.

use ubiq_proto::git::GitCommit;

/// One lane's state: the commit id it is waiting to see next, or `None` if the lane is free.
pub type Lanes = Vec<Option<String>>;

/// Assign `lane` and `merges` on every commit in `page`, in place, continuing from `lanes`.
///
/// Commits must already be in the walk's order (newest first, parents after children) — the
/// allocator has no other way to know which commit is whose parent.
///
/// The algorithm: a commit claims the lane waiting for its id, or opens the lowest free lane if
/// none is. That lane then starts waiting for the commit's first parent (or frees, for a root
/// commit). Each additional parent claims or opens its own lane — those are the commit's `merges`,
/// the columns the merge lines behind it come from.
pub fn assign_lanes(page: &mut [GitCommit], lanes: &mut Lanes) {
    for commit in page.iter_mut() {
        let lane = claim_or_open(lanes, &commit.id);
        // Anything else still waiting for this id (two branches forking from the same commit)
        // is satisfied by it too — free those lanes rather than leave them waiting forever.
        for waiting in lanes.iter_mut() {
            if waiting.as_deref() == Some(commit.id.as_str()) {
                *waiting = None;
            }
        }
        commit.lane = lane;

        let mut parents = commit.parents.iter();
        lanes[lane] = parents.next().cloned();

        let mut merges = Vec::new();
        for parent in parents {
            let extra = claim_or_open(lanes, parent);
            lanes[extra] = Some(parent.clone());
            merges.push(extra);
        }
        commit.merges = merges;
    }
}

/// The lane waiting for `id`, or the lowest free lane, opening a new one if every lane is busy.
fn claim_or_open(lanes: &mut Lanes, id: &str) -> usize {
    if let Some(found) = lanes.iter().position(|w| w.as_deref() == Some(id)) {
        return found;
    }
    if let Some(free) = lanes.iter().position(Option::is_none) {
        return free;
    }
    lanes.push(None);
    lanes.len() - 1
}
