//! The fixtures left.
//!
//! Projects, the file tree, a file's bytes, the panes, the work and the chat's conversations all
//! come from the host, and the constructors that invented them are gone. What is left is what no
//! transport family answers: the Git screen's refs and history, which the git family carries no
//! message for — `G70`. A fixture is still the honest way to draw a screen with nothing behind
//! it, and this goes the same way when it gets a family.
//!
//! The Git screen's *repository* is not here. The branch, the counts, the changed paths and the
//! diff are the host's answers; only the branch list, the tags, the stashes, the submodules and
//! the commit log are invented.

use super::git::{CommitRow, RefRow, RefSection};

/// The Git screen's sidebar: the branches, remotes, tags, stashes and submodules a project of
/// Ubiq's own shape would have.
pub fn git_refs() -> Vec<RefRow> {
    use RefSection::{Local, Remotes, Stashes, Submodules, Tags};
    vec![
        RefRow::new(Local, "main").tracking(2, 1).current(),
        RefRow::new(Local, "fix/terminal-refit").tracking(5, 0),
        RefRow::new(Local, "feat/session-store").tracking(1, 3),
        RefRow::new(Local, "spike/cold-start"),
        RefRow::new(Remotes, "origin/main"),
        RefRow::new(Remotes, "origin/fix/terminal-refit"),
        RefRow::new(Remotes, "origin/release/0.3"),
        RefRow::new(Tags, "v0.3.0"),
        RefRow::new(Tags, "v0.2.4"),
        RefRow::new(Stashes, "WIP on main: 9f3a10c panel resize"),
        RefRow::new(Submodules, "vendor/gpui-component"),
    ]
}

/// The Git screen's history, with the lanes a graph of it would draw.
pub fn git_history() -> Vec<CommitRow> {
    vec![
        commit(
            "9f3a10c",
            "Refit the terminal after a panel collapse",
            "Marco De Nittis",
            "2 h ago",
            0,
            true,
        )
        .decorated(&["main"]),
        merge(
            "4c8b221",
            "Merge branch 'feat/session-store'",
            "Marco De Nittis",
            "5 h ago",
            0,
            &[1],
        ),
        commit(
            "b1c9f30",
            "Register the session store's v2 migration",
            "Sara Villa",
            "yesterday",
            1,
            false,
        ),
        commit(
            "77de904",
            "Carry the harness id through a resume",
            "Marco De Nittis",
            "2 days ago",
            1,
            true,
        )
        .decorated(&["feat/session-store"]),
        commit(
            "1aa5c62",
            "Quote the folder in the launch line",
            "Sara Villa",
            "3 days ago",
            0,
            false,
        ),
        commit(
            "3b70e15",
            "Cut 0.3.0",
            "Marco De Nittis",
            "4 days ago",
            0,
            true,
        )
        .decorated(&["v0.3.0"]),
        commit(
            "c55b7e2",
            "Defer the first status walk until a window attaches",
            "Sara Villa",
            "5 days ago",
            2,
            false,
        ),
        commit(
            "0d41a88",
            "Add the working-tree ceiling",
            "Marco De Nittis",
            "6 days ago",
            0,
            true,
        ),
        commit(
            "e12f7b4",
            "Cut 0.2.4",
            "Marco De Nittis",
            "2 weeks ago",
            0,
            true,
        )
        .decorated(&["v0.2.4"]),
    ]
}

fn commit(
    short_id: &str,
    summary: &str,
    author: &str,
    when: &str,
    lane: usize,
    mine: bool,
) -> CommitRow {
    CommitRow {
        short_id: short_id.to_string(),
        summary: summary.to_string(),
        author: author.to_string(),
        when: when.to_string(),
        lane,
        merges: Vec::new(),
        refs: Vec::new(),
        mine,
    }
}

fn merge(
    short_id: &str,
    summary: &str,
    author: &str,
    when: &str,
    lane: usize,
    merges: &[usize],
) -> CommitRow {
    CommitRow {
        merges: merges.to_vec(),
        ..commit(short_id, summary, author, when, lane, true)
    }
}
