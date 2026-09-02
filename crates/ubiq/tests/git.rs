//! The Git screen's logic, without a frame.
//!
//! Everything the screen decides — which sections are open, which commits a search leaves, how
//! wide the graph is, which of the three lists a changed path lands in, what a click on a row
//! asks the host for, and what happens to a selection when the path it named goes clean — is
//! arithmetic over plain data, so it is tested the way the graph's and the explorer's are: on the
//! state alone, seeded the way the host would have filled it.
//!
//! The refs and the commits are the fixtures the screen ships with, because the git family carries
//! no refs list and no log. The working-tree entries are `ubiq_proto::git`'s own records, which is
//! what a `GitWorkingTree` puts on the project.

use ubiq::state::git::{GitView, RefRow, RefSection, Side, conflicted, staged, unstaged};
use ubiq::state::sample;
use ubiq_proto::files::DiffBase;
use ubiq_proto::git::{GitEntry, GitPathChange};

fn view() -> GitView {
    GitView::new(sample::git_refs(), sample::git_history())
}

fn entry(path: &str, index: Option<GitPathChange>, worktree: Option<GitPathChange>) -> GitEntry {
    GitEntry {
        rel_path: path.to_string(),
        index,
        worktree,
        conflicted: false,
        ignored: false,
    }
}

/// The working tree the change lists are read from: one staged, one staged *and* modified, one
/// untracked, one conflicted.
fn working_tree() -> Vec<GitEntry> {
    vec![
        entry("src/state/sessions.rs", Some(GitPathChange::Modified), None),
        entry(
            "src/panels/terminal.rs",
            Some(GitPathChange::Modified),
            Some(GitPathChange::Modified),
        ),
        entry("docs/architecture.md", None, Some(GitPathChange::Untracked)),
        GitEntry {
            conflicted: true,
            ..entry("Cargo.lock", None, Some(GitPathChange::Modified))
        },
    ]
}

#[test]
fn every_section_starts_open_and_shuts_on_its_own() {
    let mut git = view();
    for section in RefSection::all() {
        assert!(git.is_open(section), "{section:?} starts open");
    }

    git.toggle_section(RefSection::Tags);
    assert!(!git.is_open(RefSection::Tags));
    assert!(git.is_open(RefSection::Local), "the others are untouched");

    git.toggle_section(RefSection::Tags);
    assert!(git.is_open(RefSection::Tags));
}

#[test]
fn a_section_reports_its_own_rows() {
    let git = view();
    assert_eq!(
        git.count(RefSection::Local),
        git.rows(RefSection::Local).len()
    );
    assert!(
        git.rows(RefSection::Local)
            .iter()
            .all(|(_, row)| row.section == RefSection::Local)
    );
    // The index a row is selected by is its index in the whole list, not in its section.
    let (index, _) = git.rows(RefSection::Tags)[0];
    assert_eq!(git.refs[index].section, RefSection::Tags);
}

#[test]
fn the_screen_opens_on_the_current_branch_and_the_working_tree() {
    let git = view();
    let selected = git.selected_ref.expect("the current branch is selected");
    assert!(git.refs[selected].current);
    assert_eq!(
        git.selected_commit, None,
        "none is the uncommitted row, which is where the screen opens"
    );
}

#[test]
fn the_search_matches_summary_author_or_id() {
    let mut git = GitView::new(
        Vec::new(),
        vec![
            ubiq::state::CommitRow {
                short_id: "9f3a10c".into(),
                summary: "Refit the terminal".into(),
                author: "Sara Villa".into(),
                when: "2 h ago".into(),
                lane: 0,
                merges: Vec::new(),
                refs: Vec::new(),
                mine: false,
            },
            ubiq::state::CommitRow {
                short_id: "4c8b221".into(),
                summary: "Cut 0.3.0".into(),
                author: "Marco De Nittis".into(),
                when: "5 h ago".into(),
                lane: 0,
                merges: Vec::new(),
                refs: Vec::new(),
                mine: true,
            },
        ],
    );

    git.search = "terminal".into();
    assert_eq!(git.visible_commits().len(), 1);

    git.search = "MARCO".into();
    assert_eq!(git.visible_commits().len(), 1, "case does not matter");

    git.search = "4c8b".into();
    assert_eq!(git.visible_commits().len(), 1, "an id prefix matches");

    git.search = "  ".into();
    assert_eq!(git.visible_commits().len(), 2, "blank is not a filter");
}

#[test]
fn the_two_filters_clear_together() {
    let mut git = view();
    assert!(!git.filtered());

    git.mine_only = true;
    git.search = "cut".into();
    assert!(git.filtered());
    assert!(git.visible_commits().iter().all(|(_, c)| c.mine));

    git.clear_filters();
    assert!(!git.filtered());
    assert_eq!(git.visible_commits().len(), git.commits.len());
}

#[test]
fn the_graph_is_as_wide_as_its_widest_lane() {
    assert_eq!(GitView::new(Vec::new(), Vec::new()).lanes(), 0);
    let git = view();
    let widest = git.commits.iter().map(|c| c.lane).max().unwrap() + 1;
    assert_eq!(git.lanes(), widest);
}

#[test]
fn a_path_lands_in_a_list_for_each_side_of_its_pair() {
    let entries = working_tree();

    let staged: Vec<_> = staged(&entries)
        .iter()
        .map(|e| e.rel_path.clone())
        .collect();
    let unstaged: Vec<_> = unstaged(&entries)
        .iter()
        .map(|e| e.rel_path.clone())
        .collect();
    let conflicted: Vec<_> = conflicted(&entries)
        .iter()
        .map(|e| e.rel_path.clone())
        .collect();

    assert_eq!(
        staged,
        vec![
            "src/state/sessions.rs".to_string(),
            "src/panels/terminal.rs".to_string()
        ]
    );
    assert_eq!(
        unstaged,
        vec![
            "src/panels/terminal.rs".to_string(),
            "docs/architecture.md".to_string()
        ],
        "a path staged and modified is in both lists, which is what the pair is for"
    );
    assert_eq!(conflicted, vec!["Cargo.lock".to_string()]);
    assert!(
        !staged.contains(&"Cargo.lock".to_string()),
        "a conflicted path is only ever in its own list"
    );
}

#[test]
fn a_list_says_what_its_rows_are_compared_against() {
    assert_eq!(Side::Unstaged.base(), DiffBase::Index);
    assert_eq!(Side::Staged.base(), DiffBase::Head);
    assert_eq!(Side::Conflicted.base(), DiffBase::Head);
}

#[test]
fn picking_a_path_asks_once_and_forgets_the_last_comparison() {
    let mut git = view();

    assert!(git.select_path(Side::Unstaged, "src/panels/terminal.rs"));
    assert_eq!(git.path(), Some("src/panels/terminal.rs"));
    assert_eq!(git.base, DiffBase::Index);

    assert!(
        !git.select_path(Side::Unstaged, "src/panels/terminal.rs"),
        "the same row again is not a second question"
    );

    git.diff = None;
    assert!(git.select_path(Side::Staged, "docs/architecture.md"));
    assert_eq!(git.base, DiffBase::Head);
    assert!(
        git.diff.is_none(),
        "a comparison of the last path is never drawn under a new one"
    );
}

#[test]
fn a_selection_goes_when_its_path_goes_clean() {
    let mut git = view();
    let entries = working_tree();
    git.select_path(Side::Unstaged, "docs/architecture.md");

    git.settle(&entries);
    assert_eq!(git.path(), Some("docs/architecture.md"), "still changed");

    git.settle(&[]);
    assert_eq!(git.path(), None);
    assert!(git.diff.is_none());
}

#[test]
fn a_ref_row_carries_tracking_only_where_there_is_some() {
    let row = RefRow::new(RefSection::Local, "main").tracking(2, 0);
    assert_eq!(row.ahead, Some(2));
    assert_eq!(row.behind, None, "zero behind is drawn as nothing, not 0");
    assert!(!row.current);
    assert!(RefRow::new(RefSection::Local, "main").current().current);
}
