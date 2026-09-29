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

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq::state::git::{
    CommitRow, GitAction, GitConfirm, GitMenu, GitMenuKind, GitView, GraphCell, RefRow, RefSection,
    RefTreeKind, Side, commit_rows, conflicted, graph_cells, group_changes, ref_rows, staged,
    unstaged,
};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::files::DiffBase;
use ubiq_proto::git::{
    GitCommit, GitCounts, GitEntry, GitError, GitPathChange, GitRef, GitRefKind, GitResetMode,
    GitSubmodule, GitSubmoduleState, GitWho, GitWriteOp,
};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

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
            vec!["4c8b221".to_string()],
            true,
        ),
        row(
            "4c8b221",
            "Merge branch 'feat/session-store'",
            "Marco De Nittis",
            0,
            vec![1],
            vec!["1aa5c62".to_string(), "b1c9f30".to_string()],
            true,
        ),
        row(
            "b1c9f30",
            "Register the migration",
            "Sara Villa",
            1,
            Vec::new(),
            Vec::new(),
            false,
        ),
        row(
            "1aa5c62",
            "Cut 0.3.0",
            "Marco De Nittis",
            0,
            Vec::new(),
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
    parents: Vec<String>,
    mine: bool,
) -> CommitRow {
    CommitRow {
        id: short_id.to_string(),
        short_id: short_id.to_string(),
        summary: summary.to_string(),
        author: author.to_string(),
        when: "2 h ago".to_string(),
        lane,
        merges,
        parents,
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
fn only_local_branches_starts_open_and_every_section_shuts_on_its_own() {
    let mut git = view();
    assert!(
        git.is_open(RefSection::Local),
        "branches are what the sidebar is opened for"
    );
    for section in RefSection::all()
        .into_iter()
        .filter(|s| *s != RefSection::Local)
    {
        assert!(!git.is_open(section), "{section:?} starts shut");
    }

    git.toggle_section(RefSection::Tags);
    assert!(git.is_open(RefSection::Tags));
    assert!(git.is_open(RefSection::Local), "the others are untouched");

    git.toggle_section(RefSection::Tags);
    assert!(!git.is_open(RefSection::Tags));
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
                id: "9f3a10cabc".into(),
                short_id: "9f3a10c".into(),
                summary: "Refit the terminal".into(),
                author: "Sara Villa".into(),
                when: "2 h ago".into(),
                lane: 0,
                merges: Vec::new(),
                parents: Vec::new(),
                refs: Vec::new(),
                mine: false,
            },
            ubiq::state::CommitRow {
                id: "4c8b221abc".into(),
                short_id: "4c8b221".into(),
                summary: "Cut 0.3.0".into(),
                author: "Marco De Nittis".into(),
                when: "5 h ago".into(),
                lane: 0,
                merges: Vec::new(),
                parents: Vec::new(),
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

    git.search = "9f3a10cabc".into();
    assert_eq!(git.visible_commits().len(), 1, "the full sha matches");

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

fn cell(above: Vec<usize>, below: Vec<usize>, joins: Vec<usize>) -> GraphCell {
    GraphCell {
        above,
        below,
        joins,
    }
}

/// A straight line: every commit's only lane is the one above and the one below it, and nothing
/// converges except the commit itself. The tip has nothing above it and the root nothing below.
#[test]
fn graph_cells_trace_a_linear_history() {
    let commits = vec![
        row("c0", "tip", "a", 0, Vec::new(), vec!["c1".into()], true),
        row("c1", "middle", "a", 0, Vec::new(), vec!["c2".into()], true),
        row("c2", "root", "a", 0, Vec::new(), Vec::new(), true),
    ];
    let cells = graph_cells(&commits);

    assert_eq!(
        cells,
        vec![
            cell(Vec::new(), vec![0], Vec::new()),
            cell(vec![0], vec![0], vec![0]),
            cell(vec![0], Vec::new(), vec![0]),
        ]
    );
}

/// A merge: the first-parent line continues; the merge's extra-parent lane is born at the merge
/// row and converges at the branch tip. This is the fixture `view()` seeds.
#[test]
fn graph_cells_open_a_merge_lane_and_close_it_at_the_branch_tip() {
    let commits = vec![
        row(
            "9f3a10c",
            "Refit the terminal",
            "Marco",
            0,
            Vec::new(),
            vec!["4c8b221".into()],
            true,
        ),
        row(
            "4c8b221",
            "Merge branch 'feat/session-store'",
            "Marco",
            0,
            vec![1],
            vec!["1aa5c62".into(), "b1c9f30".into()],
            true,
        ),
        row(
            "b1c9f30",
            "Register the migration",
            "Sara",
            1,
            Vec::new(),
            Vec::new(),
            false,
        ),
        row(
            "1aa5c62",
            "Cut 0.3.0",
            "Marco",
            0,
            Vec::new(),
            Vec::new(),
            true,
        ),
    ];
    let cells = graph_cells(&commits);

    assert_eq!(
        cells,
        vec![
            cell(Vec::new(), vec![0], Vec::new()),
            cell(vec![0], vec![0, 1], vec![0]),
            cell(vec![0, 1], vec![0], vec![1]),
            cell(vec![0], Vec::new(), vec![0]),
        ]
    );
}

/// Two merges of the same branch, the older one after the newer: the newer merge opened the
/// branch's lane, so the older merge's extra-parent lane is already alive — the elbow sits on a
/// lane that passes straight through the row rather than being born there, and the older merge's
/// own dot is joined by the first-parent line it continues.
#[test]
fn graph_cells_reuse_a_merge_lane_that_is_already_alive() {
    let commits = vec![
        row(
            "merge-new",
            "Merge feature again",
            "a",
            0,
            vec![1],
            vec!["mid".into(), "branch".into()],
            true,
        ),
        row(
            "mid",
            "Mainline",
            "a",
            0,
            Vec::new(),
            vec!["merge-old".into()],
            true,
        ),
        row(
            "merge-old",
            "Merge feature",
            "a",
            0,
            vec![1],
            vec!["root".into(), "branch".into()],
            true,
        ),
        row(
            "root",
            "Mainline root",
            "a",
            0,
            Vec::new(),
            Vec::new(),
            true,
        ),
        row(
            "branch",
            "Feature tip",
            "b",
            1,
            Vec::new(),
            Vec::new(),
            true,
        ),
    ];
    let cells = graph_cells(&commits);

    assert_eq!(
        cells[2],
        cell(vec![0, 1], vec![0, 1], vec![0]),
        "the first-parent line converges into the older merge dot; the branch lane is alive too"
    );
    assert_eq!(
        cells[4],
        cell(vec![1], Vec::new(), vec![1]),
        "the branch tip closes the one line that was still waiting for it"
    );
}

/// `history()` pairs each visible commit with the cell the loaded walk gave it, in the list's own
/// order — and a search hides rows without relaying the graph out.
#[test]
fn history_keeps_the_graph_in_step_with_what_it_shows() {
    let mut git = view();
    let rows = git.history();
    assert_eq!(rows.len(), 4);
    for (index, row, cell) in &rows {
        assert_eq!(&git.graph[*index], *cell);
        assert_eq!(&git.commits[*index], *row);
    }

    git.search = "Merge".into();
    let rows = git.history();
    assert_eq!(rows.len(), 1);
    let (index, _, _) = rows[0];
    assert_eq!(git.commits[index].short_id, "4c8b221");
}

/// A same-length in-place replacement — a rebase that does not change the commit count is the
/// real-world case — must not serve a stale search haystack or a stale lane count. This is the
/// bug a cache keyed on `commits.len()` had: `set_commits` must recompute both every time, not
/// only when the length changes.
#[test]
fn a_same_length_replacement_never_serves_a_stale_haystack_or_lane_count() {
    let mut git = GitView::new(
        Vec::new(),
        vec![row(
            "aaaaaaa",
            "Refit the terminal",
            "Marco",
            0,
            Vec::new(),
            Vec::new(),
            true,
        )],
    );

    git.set_commits(vec![row(
        "bbbbbbb",
        "Register the migration",
        "Sara",
        3,
        Vec::new(),
        Vec::new(),
        false,
    )]);

    assert_eq!(git.lanes(), 4, "the new commit's lane, not the old one's");

    git.search = "terminal".into();
    assert_eq!(
        git.visible_commits().len(),
        0,
        "the old haystack must not still match after a same-length replacement"
    );

    git.search = "migration".into();
    assert_eq!(
        git.visible_commits().len(),
        1,
        "the new commit's own haystack must match"
    );
}

#[test]
fn staged_and_unstaged_helpers_still_split_the_pair() {
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
        "a path staged and modified has both sides of the pair"
    );
    assert_eq!(conflicted, vec!["Cargo.lock".to_string()]);
    assert!(
        !staged.contains(&"Cargo.lock".to_string()),
        "a conflicted path is only ever in its own list"
    );
}

#[test]
fn each_path_lands_in_exactly_one_working_tree_list() {
    let entries = working_tree();
    let groups = group_changes(&entries);
    let path = |index: usize| entries[index].rel_path.clone();

    assert_eq!(
        groups
            .conflicted
            .iter()
            .copied()
            .map(path)
            .collect::<Vec<_>>(),
        vec!["Cargo.lock".to_string()]
    );
    assert_eq!(
        groups.staged.iter().copied().map(path).collect::<Vec<_>>(),
        vec![
            "src/state/sessions.rs".to_string(),
            "src/panels/terminal.rs".to_string()
        ]
    );
    assert_eq!(
        groups
            .unstaged
            .iter()
            .copied()
            .map(path)
            .collect::<Vec<_>>(),
        vec![
            "src/panels/terminal.rs".to_string(),
            "docs/architecture.md".to_string()
        ],
        "a path staged and modified is in both lists"
    );
}

#[test]
fn a_list_says_what_its_rows_are_compared_against() {
    assert_eq!(Side::Unstaged.base(), DiffBase::Index);
    assert_eq!(Side::Staged.base(), DiffBase::Staged);
    assert_eq!(Side::Conflicted.base(), DiffBase::Head);
}

#[test]
fn picking_a_path_asks_once_and_forgets_the_last_comparison() {
    let mut git = view();

    assert!(git.select_path(Side::Unstaged, "docs/architecture.md"));
    assert_eq!(git.path(), Some("docs/architecture.md"));
    assert_eq!(git.base, DiffBase::Index);

    assert!(
        !git.select_path(Side::Unstaged, "docs/architecture.md"),
        "the same row again is not a second question"
    );

    git.diff = None;
    assert!(git.select_path(Side::Staged, "src/panels/terminal.rs"));
    assert_eq!(git.base, DiffBase::Staged);
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

#[test]
fn local_branches_nest_on_slash() {
    let git = GitView::new(
        vec![
            RefRow::new(RefSection::Local, "main").current(),
            RefRow::new(RefSection::Local, "feature/things"),
            RefRow::new(RefSection::Local, "feature/other"),
            RefRow::new(RefSection::Local, "fix/terminal-refit"),
        ],
        Vec::new(),
    );
    let tree = git.ref_tree(RefSection::Local);
    let labels: Vec<&str> = tree.iter().map(|row| row.label.as_str()).collect();
    // `main` is both a trunk and the current branch, so it sits above the folders whatever it
    // sorts as; everything under the top level stays alphabetical.
    assert_eq!(
        labels,
        vec![
            "main",
            "feature",
            "other",
            "things",
            "fix",
            "terminal-refit"
        ]
    );
    assert!(matches!(
        tree[1].kind,
        RefTreeKind::Folder {
            open: true,
            leaves: 2,
            ..
        }
    ));
    assert_eq!(tree[2].depth, 1);
    assert_eq!(tree[3].depth, 1);
}

#[test]
fn the_trunks_and_the_current_branch_sit_above_the_rest() {
    let git = GitView::new(
        vec![
            RefRow::new(RefSection::Local, "zebra"),
            RefRow::new(RefSection::Local, "develop"),
            RefRow::new(RefSection::Local, "alpha"),
            RefRow::new(RefSection::Local, "main"),
            RefRow::new(RefSection::Local, "wip").current(),
        ],
        Vec::new(),
    );
    let tree = git.ref_tree(RefSection::Local);
    let labels: Vec<&str> = tree.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(labels, vec!["wip", "main", "develop", "alpha", "zebra"]);
}

#[test]
fn remotes_nest_under_the_remote_then_the_rest() {
    let git = GitView::new(
        vec![
            RefRow::new(RefSection::Remotes, "origin/main"),
            RefRow::new(RefSection::Remotes, "origin/feature/things"),
        ],
        Vec::new(),
    );
    let tree = git.ref_tree(RefSection::Remotes);
    let labels: Vec<&str> = tree.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(labels, vec!["origin", "feature", "things", "main"]);
    assert_eq!(tree[0].depth, 0);
    assert_eq!(tree[1].depth, 1);
    assert_eq!(tree[2].depth, 2);
    assert_eq!(tree[3].depth, 1);
}

#[test]
fn a_shut_folder_hides_its_children() {
    let mut git = GitView::new(
        vec![
            RefRow::new(RefSection::Local, "feature/things"),
            RefRow::new(RefSection::Local, "feature/other"),
        ],
        Vec::new(),
    );
    git.toggle_folder(RefSection::Local, "feature");
    let tree = git.ref_tree(RefSection::Local);
    assert_eq!(tree.len(), 1);
    assert!(matches!(
        tree[0].kind,
        RefTreeKind::Folder { open: false, .. }
    ));
}

#[test]
fn a_ref_finds_the_commit_it_points_at() {
    let mut git = view();
    git.refs[0].target = "9f3a10c".into();
    git.commits[0].refs = vec!["main".into()];
    assert_eq!(git.commit_index_for_ref(0), Some(0));
}

fn who(time: i64) -> GitWho {
    GitWho {
        name: "Marco De Nittis".to_string(),
        email: "marco@example.test".to_string(),
        time,
        offset: 0,
    }
}

/// `commit_rows` no longer derives a lane or a merge marker from anything — the host's lane
/// allocator already worked out the topology, so this is a straight carry-through, not
/// arithmetic. The two rows below use different lanes and a real `merges` list precisely so a
/// regression back to "everything is lane 0" would fail here.
#[test]
fn commit_rows_carries_the_hosts_lane_and_merges_through() {
    let commits = vec![
        GitCommit {
            id: "9f3a10cabc".to_string(),
            short_id: "9f3a10c".to_string(),
            summary: "Refit the terminal".to_string(),
            author: who(1_700_000_000),
            committer: who(1_700_000_000),
            parents: vec!["parent0".to_string()],
            lane: 0,
            merges: Vec::new(),
            refs: vec!["main".to_string()],
            mine: true,
        },
        GitCommit {
            id: "4c8b221abc".to_string(),
            short_id: "4c8b221".to_string(),
            summary: "Merge branch 'feat/session-store'".to_string(),
            author: who(1_699_000_000),
            committer: who(1_699_000_000),
            parents: vec!["p1".to_string(), "p2".to_string()],
            lane: 2,
            merges: vec![0, 1],
            refs: Vec::new(),
            mine: false,
        },
    ];

    let rows = commit_rows(&commits);

    assert_eq!(rows[0].lane, 0);
    assert!(
        rows[0].merges.is_empty(),
        "a single-parent commit merges from nothing"
    );
    assert_eq!(
        rows[1].lane, 2,
        "the host's lane is carried through unchanged"
    );
    assert_eq!(
        rows[1].merges,
        vec![0, 1],
        "the host's merges are carried through unchanged, not synthesised from a parent count"
    );
    assert_eq!(rows[0].refs, vec!["main".to_string()]);
    assert!(!rows[1].mine);
}

/// The wire-level staleness rule from `receive_git`'s `GitLogPage` arm, exercised through a real
/// window and bus rather than on `commit_rows` alone, because the bug it fixes is about which
/// reply a second in-flight request discards — state `commit_rows` cannot see.
const PATIENCE: Duration = Duration::from_millis(500);

struct Fixture {
    state: Entity<AppState>,
    window: WindowHandle<Root>,
    host: bus::HostEnd,
    project: ProjectId,
}

impl Fixture {
    fn open(cx: &mut TestAppContext) -> Self {
        let snapshot = a_project();
        let project = snapshot.record.id;
        let (hub, host) = bus::hub();

        cx.update(|cx| {
            gpui_component::init(cx);
            ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
            BusHub::install(hub, cx);
            WindowRegistry::install(cx);
            cx.global_mut::<WindowRegistry>().apply(snapshot);
        });

        let held: Rc<RefCell<Option<Entity<AppState>>>> = Rc::default();
        let taken = held.clone();
        let window = cx.add_window(move |window, cx| {
            let state = cx.new(|cx| AppState::for_project(Some(project), 'A', window, cx));
            *taken.borrow_mut() = Some(state.clone());
            Root::new(state, window, cx)
        });
        cx.run_until_parked();

        let state = held
            .borrow_mut()
            .take()
            .expect("the window built its state");
        Self {
            state,
            window,
            host,
            project,
        }
    }

    /// Everything the window has said so far, in order. Draining it is how a test gets past the
    /// burst of requests a fresh project fires on open.
    fn said(&self) -> Vec<Message> {
        let mut said = Vec::new();
        while let Ok(event) = self.host.recv_timeout(PATIENCE) {
            if let FromClient::Said { message, .. } = event {
                said.push(message);
            }
        }
        said
    }

    fn deliver(&self, message: Message, cx: &mut TestAppContext) {
        self.host.send(To::Everyone, message);
        cx.run_until_parked();
    }

    fn refresh_git(&self, cx: &mut TestAppContext) {
        self.window
            .update(cx, |_, _window, cx| {
                self.state.update(cx, |state, cx| state.refresh_git(cx));
            })
            .expect("the window is open");
    }

    fn stage_git_path(&self, path: &str, cx: &mut TestAppContext) {
        self.window
            .update(cx, |_, _window, cx| {
                self.state
                    .update(cx, |state, cx| state.stage_git_path(path, cx));
            })
            .expect("the window is open");
    }

    fn commit_git(&self, message: &str, cx: &mut TestAppContext) {
        self.window
            .update(cx, |_, window, cx| {
                self.state.update(cx, |state, cx| {
                    let field = state.git_message.clone();
                    field.update(cx, |input, cx| input.set_value(message, window, cx));
                    state.commit_git(cx);
                });
            })
            .expect("the window is open");
    }

    fn fetch_all_git(&self, cx: &mut TestAppContext) {
        self.window
            .update(cx, |_, _window, cx| {
                self.state.update(cx, |state, cx| state.fetch_all_git(cx));
            })
            .expect("the window is open");
    }

    fn pull_git(&self, cx: &mut TestAppContext) {
        self.window
            .update(cx, |_, _window, cx| {
                self.state.update(cx, |state, cx| state.pull_git(cx));
            })
            .expect("the window is open");
    }

    fn push_git(&self, cx: &mut TestAppContext) {
        self.window
            .update(cx, |_, _window, cx| {
                self.state.update(cx, |state, cx| state.push_git(cx));
            })
            .expect("the window is open");
    }

    fn commits(&self, cx: &mut TestAppContext) -> Vec<String> {
        self.window
            .update(cx, |_, _window, cx| {
                self.state.read(cx).git_view(cx).map_or(Vec::new(), |git| {
                    git.commits.iter().map(|c| c.short_id.clone()).collect()
                })
            })
            .expect("the window is open")
    }
}

fn a_project() -> ProjectSnapshot {
    ProjectSnapshot {
        record: ProjectRecord {
            id: ProjectId::generate(),
            name: "ubiq".to_string(),
            path: "/tmp/ubiq".to_string(),
            colour: 0,
            custom_colour: None,
            storage: Default::default(),
            temporary: false,
            created_at: Utc::now(),
            last_opened_at: None,
            search_excludes: Vec::new(),
            index: None,
            mission_term: None,
            tools: Vec::new(),
            managed_repos: Vec::new(),
            lanes: Vec::new(),
            runs_on: None,
            initials: String::new(),
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

fn commit(short_id: &str) -> GitCommit {
    GitCommit {
        id: format!("{short_id}full"),
        short_id: short_id.to_string(),
        summary: "a commit".to_string(),
        author: who(1_700_000_000),
        committer: who(1_700_000_000),
        parents: Vec::new(),
        lane: 0,
        merges: Vec::new(),
        refs: Vec::new(),
        mine: false,
    }
}

/// `G128`: two first-page requests can be in flight together — the ordinary way is a refresh
/// fired while the project's own opening request has not answered yet. If the later request's
/// reply lands first, the earlier one's reply must be discarded when it finally arrives, not
/// appended onto what the later reply already replaced it with — that append is the doubling bug.
#[gpui::test]
fn a_stale_first_page_reply_is_discarded_not_appended(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    // Drain the burst of requests a fresh project fires on open, which includes the first
    // `ProjectGitLog` — the "earlier" request this test's stale reply answers.
    let _ = fixture.said();

    // A refresh fires a second, later `ProjectGitLog` before the first one has answered.
    fixture.refresh_git(cx);
    let _ = fixture.said();

    // The later request's reply lands first and replaces.
    fixture.deliver(
        Message::GitLogPage {
            project_id: fixture.project,
            repo: String::new(),
            cursor: None,
            commits: vec![commit("later")],
            next_cursor: Some("later-cursor".to_string()),
        },
        cx,
    );
    assert_eq!(fixture.commits(cx), vec!["later".to_string()]);

    // The earlier request's reply lands after. Its own cursor (`None`) is identical to the
    // later reply's — the doubling bug's whole premise — so only the view's own bookkeeping of
    // which request is still outstanding can tell them apart.
    fixture.deliver(
        Message::GitLogPage {
            project_id: fixture.project,
            repo: String::new(),
            cursor: None,
            commits: vec![commit("earlier")],
            next_cursor: Some("earlier-cursor".to_string()),
        },
        cx,
    );

    assert_eq!(
        fixture.commits(cx),
        vec!["later".to_string()],
        "the stale reply is discarded, not appended onto the reply that already superseded it"
    );
}

#[gpui::test]
fn write_actions_send_the_matching_ops(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.stage_git_path("src/panels/terminal.rs", cx);
    let said = fixture.said();
    assert!(
        said.iter().any(|message| matches!(
            message,
            Message::WriteProjectGit {
                op: GitWriteOp::Stage { rel_path },
                ..
            } if rel_path == "src/panels/terminal.rs"
        )),
        "stage sends WriteProjectGit::Stage; got {said:?}"
    );

    fixture.commit_git("Refit the terminal", cx);
    let said = fixture.said();
    assert!(
        said.iter().any(|message| matches!(
            message,
            Message::WriteProjectGit {
                op: GitWriteOp::Commit { message, amend: false },
                ..
            } if message == "Refit the terminal"
        )),
        "commit sends WriteProjectGit::Commit; got {said:?}"
    );

    fixture.fetch_all_git(cx);
    let said = fixture.said();
    assert!(
        said.iter().any(|message| matches!(
            message,
            Message::WriteProjectGit {
                op: GitWriteOp::FetchAll,
                ..
            }
        )),
        "fetch all sends WriteProjectGit::FetchAll; got {said:?}"
    );

    fixture.pull_git(cx);
    let said = fixture.said();
    assert!(
        said.iter().any(|message| matches!(
            message,
            Message::WriteProjectGit {
                op: GitWriteOp::Pull,
                ..
            }
        )),
        "pull sends WriteProjectGit::Pull; got {said:?}"
    );

    fixture.push_git(cx);
    let said = fixture.said();
    assert!(
        said.iter().any(|message| matches!(
            message,
            Message::WriteProjectGit {
                op: GitWriteOp::Push,
                ..
            }
        )),
        "push sends WriteProjectGit::Push; got {said:?}"
    );
}

// ── a project that holds several repositories ───────────────────────────────

fn overview(
    head: &str,
    counts: Option<ubiq_proto::git::GitCounts>,
) -> ubiq_proto::git::RepoOverview {
    ubiq_proto::git::RepoOverview {
        scoped_to: String::new(),
        head: ubiq_proto::git::GitHead::Branch(head.to_string()),
        upstream: None,
        ahead: Some(2),
        behind: None,
        operation: None,
        counts,
        is_bare: false,
        generation: 1,
        remotes: Vec::new(),
        submodules: Vec::new(),
    }
}

fn nested_repo(path: &str, managed: bool) -> ubiq_proto::git::GitNested {
    ubiq_proto::git::GitNested {
        rel_path: path.to_string(),
        head: ubiq_proto::git::GitHead::Branch("main".to_string()),
        submodule: false,
        counts: Some(ubiq_proto::git::GitCounts {
            staged: 0,
            modified: 1,
            untracked: 0,
            conflicted: 1,
        }),
        ahead: None,
        behind: Some(3),
        managed,
    }
}

#[test]
fn the_repositories_section_lists_managed_repositories_and_hides_for_one() {
    use ubiq::state::git::repo_entries;

    let root = overview("main", None);
    // One repository is nothing to choose between.
    assert!(repo_entries(Some(&root), &[], "", "ubiq").is_empty());
    assert!(repo_entries(Some(&root), &[nested_repo("vendor/lib", false)], "", "ubiq").is_empty());
    assert!(
        repo_entries(
            None,
            &[nested_repo("vendor/lib", true)],
            "vendor/lib",
            "ubiq"
        )
        .is_empty()
    );

    let rows = repo_entries(
        Some(&root),
        &[
            nested_repo("vendor/lib", true),
            nested_repo("vendor/skip", false),
        ],
        "vendor/lib",
        "ubiq",
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (rows[0].label.as_str(), rows[0].selected, rows[0].ahead),
        ("ubiq", false, 2)
    );
    let lib = &rows[1];
    assert_eq!(lib.repo, "vendor/lib");
    assert!(lib.selected);
    assert_eq!(
        (lib.changes, lib.conflicted, lib.ahead, lib.behind),
        (1, 1, 0, 3)
    );
}

#[gpui::test]
fn the_screen_follows_the_repository_it_was_pointed_at(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.deliver(
        Message::GitOverview {
            project_id: fixture.project,
            repo: String::new(),
            overview: Some(overview("main", None)),
        },
        cx,
    );
    fixture.deliver(
        Message::GitWorkingTree {
            project_id: fixture.project,
            generation: 1,
            entries: vec![
                entry("a.txt", None, Some(GitPathChange::Modified)),
                entry("vendor/lib/b.txt", None, Some(GitPathChange::Untracked)),
            ],
            rollups: Vec::new(),
            repos: vec![nested_repo("vendor/lib", true)],
            truncated: false,
        },
        cx,
    );
    let paths = |cx: &mut TestAppContext| -> Vec<String> {
        fixture.state.read_with(cx, |state, cx| {
            state
                .git_entries(cx)
                .unwrap_or(&[])
                .iter()
                .map(|entry| entry.rel_path.clone())
                .collect()
        })
    };
    assert_eq!(
        paths(cx),
        vec!["a.txt".to_string()],
        "the project's own list drops the nested repository's paths"
    );

    fixture
        .window
        .update(cx, |_, _window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.select_git_repo("vendor/lib".into(), cx)
            });
        })
        .expect("the window is open");
    assert_eq!(paths(cx), vec!["vendor/lib/b.txt".to_string()]);
    let said = fixture.said();
    for wanted in ["overview", "refs", "log"] {
        assert!(
            said.iter().any(|message| match message {
                Message::ProjectGit { repo, .. } => wanted == "overview" && repo == "vendor/lib",
                Message::ProjectGitRefs { repo, .. } => wanted == "refs" && repo == "vendor/lib",
                Message::ProjectGitLog { repo, .. } => wanted == "log" && repo == "vendor/lib",
                _ => false,
            }),
            "selecting a repository asks for its {wanted}; got {said:?}"
        );
    }

    // What the repository the user left answers is not the screen's any more.
    fixture.deliver(
        Message::GitLogPage {
            project_id: fixture.project,
            repo: String::new(),
            cursor: None,
            commits: vec![commit("root")],
            next_cursor: None,
        },
        cx,
    );
    assert!(fixture.commits(cx).is_empty());
    fixture.deliver(
        Message::GitLogPage {
            project_id: fixture.project,
            repo: "vendor/lib".to_string(),
            cursor: None,
            commits: vec![commit("nested")],
            next_cursor: None,
        },
        cx,
    );
    assert_eq!(fixture.commits(cx), vec!["nested".to_string()]);

    // A write goes to the repository on screen.
    fixture.stage_git_path("vendor/lib/b.txt", cx);
    assert!(fixture.said().iter().any(|message| matches!(
        message,
        Message::WriteProjectGit { repo, .. } if repo == "vendor/lib"
    )));
}

// ── T-269: checkout affordances, the dead menu rows, and revert-to-commit ──

fn ref_row_kind(name: &str, kind: GitRefKind, current: bool) -> GitRef {
    GitRef {
        name: name.to_string(),
        kind,
        target: "abc123d".to_string(),
        current,
        ahead: None,
        behind: None,
    }
}

/// What every one of the `Ref` menu's three write rows offers, by section and by whether the row
/// is the one HEAD is on — the table T-269 asks for: only a local branch or a tag can be merged
/// or deleted, a checkout is offered more broadly, and merging into the branch already checked
/// out is never offered.
#[test]
fn ref_menu_gates_checkout_merge_delete_by_section_and_current() {
    let refs = vec![
        RefRow::new(RefSection::Local, "main").current(),
        RefRow::new(RefSection::Local, "feature"),
        RefRow::new(RefSection::Remotes, "origin/main"),
        RefRow::new(RefSection::Tags, "v1.0"),
        RefRow::new(RefSection::Stashes, "WIP on main"),
        RefRow::new(RefSection::Submodules, "vendor/lib"),
    ];
    let enabled = |index: usize, action: GitAction| -> bool {
        GitMenu {
            epoch: 0,
            kind: GitMenuKind::Ref { index },
            x: 0.,
            y: 0.,
        }
        .entries(false, false, &refs, false)
        .into_iter()
        .find(|entry| entry.action == action)
        .expect("the row is drawn")
        .enabled
    };

    // The checked-out local branch: checkout is offered, merging it into itself is not, delete
    // stays live.
    assert!(enabled(0, GitAction::Checkout));
    assert!(!enabled(0, GitAction::Merge));
    assert!(enabled(0, GitAction::Delete));

    // Another local branch: all three.
    assert!(enabled(1, GitAction::Checkout));
    assert!(enabled(1, GitAction::Merge));
    assert!(enabled(1, GitAction::Delete));

    // A remote-tracking branch: checkout only.
    assert!(enabled(2, GitAction::Checkout));
    assert!(!enabled(2, GitAction::Merge));
    assert!(!enabled(2, GitAction::Delete));

    // A tag: all three, the same as a local branch.
    assert!(enabled(3, GitAction::Checkout));
    assert!(enabled(3, GitAction::Merge));
    assert!(enabled(3, GitAction::Delete));

    // A stash and a submodule row: none of the three.
    for dead in [4, 5] {
        assert!(!enabled(dead, GitAction::Checkout), "row {dead}");
        assert!(!enabled(dead, GitAction::Merge), "row {dead}");
        assert!(!enabled(dead, GitAction::Delete), "row {dead}");
    }
}

/// The Change menu's `Discard` and `Open` are always offered on the row that raised them, and
/// `Revert to commit` only when the history panel has a real commit selected. The Commit menu's
/// three actions and three reset modes are always offered — the host is what refuses a `Reset`
/// or a `CherryPick` that does not make sense, on the same rule stage/unstage already follow.
#[test]
fn change_and_commit_menu_entries() {
    let change = GitMenu {
        epoch: 0,
        kind: GitMenuKind::Change {
            path: "a.txt".to_string(),
            side: Side::Unstaged,
        },
        x: 0.,
        y: 0.,
    };
    let get = |entries: &[ubiq::state::git::GitMenuEntry], action: GitAction| {
        entries
            .iter()
            .find(|entry| entry.action == action)
            .expect("the row is drawn")
            .enabled
    };
    let no_commit = change.entries(true, false, &[], false);
    assert!(get(&no_commit, GitAction::Discard));
    assert!(get(&no_commit, GitAction::Open));
    assert!(!get(&no_commit, GitAction::RevertToCommit));

    let with_commit = change.entries(true, false, &[], true);
    assert!(get(&with_commit, GitAction::RevertToCommit));

    let commit = GitMenu {
        epoch: 0,
        kind: GitMenuKind::Commit { index: 0 },
        x: 0.,
        y: 0.,
    };
    let entries = commit.entries(false, false, &[], false);
    for action in [
        GitAction::CherryPick,
        GitAction::Revert,
        GitAction::ResetSoft,
        GitAction::ResetMixed,
        GitAction::ResetHard,
    ] {
        assert!(get(&entries, action), "{action:?} should be live");
    }
}

/// The one path every checkout affordance takes: **always unforced first**, whatever the working
/// tree holds. A dirty tree whose changes do not collide comes across the switch, so asking on
/// dirtiness alone would turn every switch into a discard; the confirm is raised from the host's
/// refusal instead, and only its accept sends `force: true`. A refusal that is not about
/// overwriting work reads as the screen's error and offers nothing.
#[gpui::test]
fn a_dirty_checkout_is_unforced_until_the_host_refuses(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.deliver(
        Message::GitRefs {
            project_id: fixture.project,
            repo: String::new(),
            refs: vec![
                ref_row_kind("main", GitRefKind::Local, true),
                ref_row_kind("feature", GitRefKind::Local, false),
            ],
        },
        cx,
    );
    // Everything is dirty: staged, modified, untracked and conflicted all count.
    fixture.deliver(
        Message::GitOverview {
            project_id: fixture.project,
            repo: String::new(),
            overview: Some(overview(
                "main",
                Some(GitCounts {
                    staged: 1,
                    modified: 2,
                    untracked: 3,
                    conflicted: 0,
                }),
            )),
        },
        cx,
    );

    let checkout = |index: usize, cx: &mut TestAppContext| {
        fixture
            .window
            .update(cx, |_, _window, cx| {
                fixture
                    .state
                    .update(cx, |state, cx| state.checkout_git_ref(index, cx));
            })
            .expect("the window is open");
    };
    let confirm_state = |cx: &mut TestAppContext| {
        fixture.state.read_with(cx, |state, cx| {
            state.git_view(cx).and_then(|git| git.confirm.clone())
        })
    };
    let last_error = |cx: &mut TestAppContext| {
        fixture.state.read_with(cx, |state, cx| {
            state.git_view(cx).and_then(|git| git.last_error.clone())
        })
    };
    let fail = |reason: &str, cx: &mut TestAppContext| {
        fixture.deliver(
            Message::GitError {
                project_id: fixture.project,
                repo: String::new(),
                error: GitError::Failed(reason.to_string()),
            },
            cx,
        );
    };

    // The dirty tree writes unforced and asks nothing — this is the bug the card is about.
    checkout(1, cx);
    let said = fixture.said();
    assert!(
        said.iter().any(|message| matches!(
            message,
            Message::WriteProjectGit {
                op: GitWriteOp::Checkout { rev, force: false },
                ..
            } if rev == "feature"
        )),
        "a dirty tree still checks out unforced; got {said:?}"
    );
    assert!(
        !said.iter().any(|message| matches!(
            message,
            Message::WriteProjectGit {
                op: GitWriteOp::Checkout { force: true, .. },
                ..
            }
        )),
        "nothing forced is sent before the host has refused; got {said:?}"
    );
    assert_eq!(confirm_state(cx), None);

    // The host refuses it because the switch would overwrite work: *that* is the question.
    fail("1 conflict prevents checkout", cx);
    assert_eq!(
        confirm_state(cx),
        Some(GitConfirm::Checkout {
            rev: "feature".to_string()
        })
    );
    assert_eq!(
        last_error(cx),
        None,
        "the refusal is a question, not an error"
    );

    // Accepting is the only thing that forces it.
    fixture
        .window
        .update(cx, |_, _window, cx| {
            fixture
                .state
                .update(cx, |state, cx| state.confirm_git_action(cx));
        })
        .expect("the window is open");
    let said = fixture.said();
    assert!(
        said.iter().any(|message| matches!(
            message,
            Message::WriteProjectGit {
                op: GitWriteOp::Checkout { rev, force: true },
                ..
            } if rev == "feature"
        )),
        "confirming sends the forced checkout; got {said:?}"
    );
    assert_eq!(confirm_state(cx), None);

    // A checkout that fails for any other reason is an error, not an offer to force one.
    checkout(1, cx);
    let _ = fixture.said();
    fail("'feature' is not a valid rev", cx);
    assert_eq!(confirm_state(cx), None);
    assert_eq!(last_error(cx), Some("'feature' is not a valid rev".into()));

    // And a conflict landing when no checkout is outstanding is nobody's question either.
    fail("1 conflict prevents checkout", cx);
    assert_eq!(confirm_state(cx), None);
}

/// The Ref menu's `Merge` is dead on every row while `HEAD` is detached — no row is `current`,
/// and there is no branch to merge onto, which the host refuses outright. Checkout and delete are
/// unaffected.
#[test]
fn ref_menu_kills_merge_on_a_detached_head() {
    let refs = vec![
        RefRow::new(RefSection::Local, "main"),
        RefRow::new(RefSection::Tags, "v1.0"),
    ];
    let enabled = |index: usize, action: GitAction| -> bool {
        GitMenu {
            epoch: 0,
            kind: GitMenuKind::Ref { index },
            x: 0.,
            y: 0.,
        }
        .entries(false, false, &refs, false)
        .into_iter()
        .find(|entry| entry.action == action)
        .expect("the row is drawn")
        .enabled
    };
    for row in [0, 1] {
        assert!(!enabled(row, GitAction::Merge), "row {row}");
        assert!(enabled(row, GitAction::Checkout), "row {row}");
        assert!(enabled(row, GitAction::Delete), "row {row}");
    }
}

/// Every menu row T-269 asked to be enabled sends the write it names — the confirm-gated ones
/// (`Discard`, `Revert to commit`, `Delete`) only once answered, the rest straight away. `Open` is
/// a UI action, not a
/// write: it asks the host to read the file, the same request a plain click in the explorer
/// sends.
#[gpui::test]
fn menu_actions_write_what_they_name(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.deliver(
        Message::GitRefs {
            project_id: fixture.project,
            repo: String::new(),
            refs: vec![
                ref_row_kind("main", GitRefKind::Local, true),
                ref_row_kind("feature", GitRefKind::Local, false),
            ],
        },
        cx,
    );
    fixture.deliver(
        Message::GitLogPage {
            project_id: fixture.project,
            repo: String::new(),
            cursor: None,
            commits: vec![commit("4f2a9c1")],
            next_cursor: None,
        },
        cx,
    );
    fixture.deliver(
        Message::GitWorkingTree {
            project_id: fixture.project,
            generation: 1,
            entries: vec![entry("a.txt", None, Some(GitPathChange::Modified))],
            rollups: Vec::new(),
            repos: Vec::new(),
            truncated: false,
        },
        cx,
    );

    let open_menu = |kind: GitMenuKind, cx: &mut TestAppContext| {
        fixture
            .window
            .update(cx, |_, _window, cx| {
                fixture
                    .state
                    .update(cx, |state, cx| state.open_git_menu(kind, (0., 0.), cx));
            })
            .expect("the window is open");
    };
    let pick = |index: usize, cx: &mut TestAppContext| {
        fixture
            .window
            .update(cx, |_, _window, cx| {
                fixture
                    .state
                    .update(cx, |state, cx| state.pick_git_menu_action(index, cx));
            })
            .expect("the window is open");
    };
    let confirm = |cx: &mut TestAppContext| {
        fixture
            .window
            .update(cx, |_, _window, cx| {
                fixture
                    .state
                    .update(cx, |state, cx| state.confirm_git_action(cx));
            })
            .expect("the window is open");
    };
    let change = || GitMenuKind::Change {
        path: "a.txt".to_string(),
        side: Side::Unstaged,
    };

    // `Discard`: index 3 of the Change menu, behind a confirm — it destroys work.
    open_menu(change(), cx);
    pick(3, cx);
    assert!(fixture.said().is_empty(), "Discard waits on the confirm");
    confirm(cx);
    assert!(fixture.said().iter().any(|message| matches!(
        message,
        Message::WriteProjectGit { op: GitWriteOp::Discard { rel_path }, .. } if rel_path == "a.txt"
    )));

    // `Open`: index 4, a UI read, not a write — the same request a click in the explorer sends.
    open_menu(change(), cx);
    pick(4, cx);
    assert!(fixture.said().iter().any(|message| matches!(
        message,
        Message::ReadProjectFile { rel_path, .. } if rel_path == "a.txt"
    )));

    // `Revert to commit`: index 5, disabled with nothing selected in the history — selecting the
    // seeded commit turns it live. It overwrites the working tree with content that is in no
    // commit, so like `Discard` it waits on the confirm, then restores from that commit's full id.
    open_menu(change(), cx);
    pick(5, cx);
    assert!(
        fixture.said().is_empty(),
        "Revert to commit is dead with no commit selected"
    );
    fixture
        .window
        .update(cx, |_, _window, cx| {
            fixture
                .state
                .update(cx, |state, cx| state.select_git_commit(Some(0), cx));
        })
        .expect("the window is open");
    open_menu(change(), cx);
    pick(5, cx);
    assert!(
        fixture.said().is_empty(),
        "Revert to commit waits on the confirm"
    );
    confirm(cx);
    assert!(fixture.said().iter().any(|message| matches!(
        message,
        Message::WriteProjectGit {
            op: GitWriteOp::RestoreFile { rel_path, rev },
            ..
        } if rel_path == "a.txt" && rev == "4f2a9c1full"
    )));

    // The Commit menu: cherry-pick, revert, and the two unconfirmed reset modes.
    let commit_menu = || GitMenuKind::Commit { index: 0 };
    open_menu(commit_menu(), cx);
    pick(2, cx); // CherryPick
    assert!(fixture.said().iter().any(|message| matches!(
        message,
        Message::WriteProjectGit { op: GitWriteOp::CherryPick { sha }, .. } if sha == "4f2a9c1full"
    )));

    open_menu(commit_menu(), cx);
    pick(3, cx); // Revert
    assert!(fixture.said().iter().any(|message| matches!(
        message,
        Message::WriteProjectGit { op: GitWriteOp::RevertCommit { sha }, .. } if sha == "4f2a9c1full"
    )));

    open_menu(commit_menu(), cx);
    pick(5, cx); // Reset (soft) — no confirm
    assert!(fixture.said().iter().any(|message| matches!(
        message,
        Message::WriteProjectGit {
            op: GitWriteOp::Reset { sha, mode: GitResetMode::Soft },
            ..
        } if sha == "4f2a9c1full"
    )));

    open_menu(commit_menu(), cx);
    pick(7, cx); // Reset (hard) — behind a confirm
    assert!(
        fixture.said().is_empty(),
        "a hard reset waits on the confirm"
    );
    confirm(cx);
    assert!(fixture.said().iter().any(|message| matches!(
        message,
        Message::WriteProjectGit {
            op: GitWriteOp::Reset { sha, mode: GitResetMode::Hard },
            ..
        } if sha == "4f2a9c1full"
    )));

    // The Ref menu: merge the other branch, then delete it behind a confirm.
    open_menu(GitMenuKind::Ref { index: 1 }, cx);
    pick(1, cx); // Merge
    assert!(fixture.said().iter().any(|message| matches!(
        message,
        Message::WriteProjectGit { op: GitWriteOp::Merge { rev }, .. } if rev == "feature"
    )));

    // `Delete`: behind the confirm, and *forced* once it is answered — the dialog said the branch
    // and its unmerged commits go, so an unforced write the host would refuse would leave the
    // question answered and nothing done.
    open_menu(GitMenuKind::Ref { index: 1 }, cx);
    pick(2, cx); // Delete
    assert!(fixture.said().is_empty(), "Delete waits on the confirm");
    confirm(cx);
    assert!(fixture.said().iter().any(|message| matches!(
        message,
        Message::WriteProjectGit {
            op: GitWriteOp::DeleteRef { name, force: true },
            ..
        } if name == "feature"
    )));
}
