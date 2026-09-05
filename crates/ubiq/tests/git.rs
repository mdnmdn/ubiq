//! The Git screen's logic, without a frame.
//!
//! Everything the screen decides — which sections are open, which commits a search leaves, how
//! wide the graph is, which of the three lists a changed path lands in, what a click on a row
//! asks the host for, and what happens to a selection when the path it named goes clean — is
//! arithmetic over plain data, so it is tested the way the graph's and the explorer's are: on the
//! state alone, seeded the way the host would have filled it.
//!
//! The working-tree entries are `ubiq_proto::git`'s own records, which is what a `GitWorkingTree`
//! puts on the project. The refs and the commits below are seeded directly as `RefRow`/`CommitRow`
//! — what a `GitRefs`/`GitLogPage` reply would already have been turned into by `ref_rows` and
//! `commit_rows`, which have their own tests further down.

use ubiq::state::git::{
    CommitRow, GitView, RefRow, RefSection, Side, commit_rows, conflicted, ref_rows, staged,
    unstaged,
};
use ubiq_proto::files::DiffBase;
use ubiq_proto::git::{
    GitCommit, GitEntry, GitPathChange, GitRef, GitRefKind, GitSubmodule, GitSubmoduleState, GitWho,
};

/// A sidebar and a history with one row of every kind the fixtures used to seed, so the
/// behavioural tests below (sections, search, lanes, tracking) exercise the same shapes.
fn view() -> GitView {
    use RefSection::{Local, Remotes, Stashes, Submodules, Tags};
    let refs = vec![
        RefRow::new(Local, "main").tracking(2, 1).current(),
        RefRow::new(Local, "fix/terminal-refit").tracking(5, 0),
        RefRow::new(Remotes, "origin/main"),
        RefRow::new(Tags, "v0.3.0"),
        RefRow::new(Stashes, "WIP on main: 9f3a10c panel resize"),
        RefRow::new(Submodules, "vendor/gpui-component"),
    ];
    let commits = vec![
        row(
            "9f3a10c",
            "Refit the terminal",
            "Marco De Nittis",
            0,
            Vec::new(),
            true,
        ),
        row(
            "4c8b221",
            "Merge branch 'feat/session-store'",
            "Marco De Nittis",
            0,
            vec![1],
            true,
        ),
        row(
            "b1c9f30",
            "Register the migration",
            "Sara Villa",
            1,
            Vec::new(),
            false,
        ),
        row(
            "1aa5c62",
            "Cut 0.3.0",
            "Marco De Nittis",
            0,
            Vec::new(),
            true,
        ),
    ];
    GitView::new(refs, commits)
}

fn row(
    short_id: &str,
    summary: &str,
    author: &str,
    lane: usize,
    merges: Vec<usize>,
    mine: bool,
) -> CommitRow {
    CommitRow {
        short_id: short_id.to_string(),
        summary: summary.to_string(),
        author: author.to_string(),
        when: "2 h ago".to_string(),
        lane,
        merges,
        refs: Vec::new(),
        mine,
    }
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

fn git_ref(name: &str, kind: GitRefKind, current: bool) -> GitRef {
    GitRef {
        name: name.to_string(),
        kind,
        target: "abc1234".to_string(),
        current,
        ahead: None,
        behind: None,
    }
}

#[test]
fn ref_rows_sorts_into_sections_and_marks_the_current_row() {
    let refs = vec![
        git_ref("main", GitRefKind::Local, true),
        git_ref("origin/main", GitRefKind::Remote, false),
        git_ref("v0.3.0", GitRefKind::Tag, false),
        git_ref("stash@{0}", GitRefKind::Stash, false),
    ];
    let submodules = vec![GitSubmodule {
        name: "gpui-component".to_string(),
        rel_path: "vendor/gpui-component".to_string(),
        url: "https://example/gpui-component".to_string(),
        state: GitSubmoduleState::Clean,
    }];

    let rows = ref_rows(&refs, &submodules);

    assert_eq!(
        rows.iter().map(|r| r.section).collect::<Vec<_>>(),
        vec![
            RefSection::Local,
            RefSection::Remotes,
            RefSection::Tags,
            RefSection::Stashes,
            RefSection::Submodules,
        ]
    );
    assert_eq!(rows.iter().filter(|r| r.current).count(), 1);
    assert!(rows.iter().find(|r| r.name == "main").unwrap().current);
    assert_eq!(
        rows.last().unwrap().name,
        "vendor/gpui-component",
        "a submodule's row is its project-relative path"
    );
}

fn who(time: i64) -> GitWho {
    GitWho {
        name: "Marco De Nittis".to_string(),
        email: "marco@example.test".to_string(),
        time,
        offset: 0,
    }
}

#[test]
fn commit_rows_marks_a_two_parent_commit_as_a_merge() {
    let commits = vec![
        GitCommit {
            id: "9f3a10cabc".to_string(),
            short_id: "9f3a10c".to_string(),
            summary: "Refit the terminal".to_string(),
            author: who(1_700_000_000),
            committer: who(1_700_000_000),
            parents: 1,
            refs: vec!["main".to_string()],
            mine: true,
        },
        GitCommit {
            id: "4c8b221abc".to_string(),
            short_id: "4c8b221".to_string(),
            summary: "Merge branch 'feat/session-store'".to_string(),
            author: who(1_699_000_000),
            committer: who(1_699_000_000),
            parents: 2,
            refs: Vec::new(),
            mine: false,
        },
    ];

    let rows = commit_rows(&commits);

    assert!(rows[0].merges.is_empty(), "one parent is not a merge");
    assert!(!rows[1].merges.is_empty(), "two parents draws a hollow dot");
    assert_eq!(rows[0].refs, vec!["main".to_string()]);
    assert!(!rows[1].mine);
}
