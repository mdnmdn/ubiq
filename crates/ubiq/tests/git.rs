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
    CommitRow, GitView, RefRow, RefSection, Side, commit_rows, conflicted, ref_rows, staged,
    unstaged,
};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::files::DiffBase;
use ubiq_proto::git::{
    GitCommit, GitEntry, GitPathChange, GitRef, GitRefKind, GitSubmodule, GitSubmoduleState, GitWho,
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
            temporary: false,
            created_at: Utc::now(),
            last_opened_at: None,
            search_excludes: Vec::new(),
            no_local_index: false,
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
