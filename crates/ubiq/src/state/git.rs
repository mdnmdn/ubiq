//! The Git screen's own view of a project's repository: which sidebar sections are open, what is
//! selected in the history, which changed path the diff under it is about, and what is typed in
//! the commit box.
//!
//! **The repository itself is not here.** What the host has observed arrives as a
//! [`ubiq_proto::git::RepoOverview`] and a list of [`GitEntry`] pairs, and lives on the project the
//! way the work does; this is the view over them, which is why every reader that concerns the
//! working tree takes the entries as its first parameter rather than holding them. The split is
//! what keeps both halves testable without a frame, and it is the same shape `GraphView`'s readers
//! have.
//!
//! [`RefRow`] and [`CommitRow`] are the sidebar's and the history's own rows, built from the
//! host's [`ubiq_proto::git::GitRef`]/[`ubiq_proto::git::GitSubmodule`] and
//! [`ubiq_proto::git::GitCommit`] answers by [`ref_rows`] and [`commit_rows`]. The lane a commit
//! draws in and the lanes it merges from are the host's own answer, computed there by its lane
//! allocator over real parent ids. What the interface adds is [`graph_cells`]: the per-row shape
//! of those lanes — which are live above and below, and which end at this commit — a projection
//! of the host's `lane`/`merges`/`parents`, deterministic over the same data, so two windows
//! still cannot draw the same history differently. Everything else on the screen is the host's
//! answer too: the
//! branch, the ahead and behind counts, the in-progress
//! operation, the working-tree totals, the conflicted / staged / unstaged lists, and the
//! diff under them.
//!
//! **This screen writes.** Stage and unstage run through `+` / `-` on each path; commit, fetch
//! all, pull and push are live. Branch, stash and undo stay inert. What is typed into the commit
//! box is kept here so a switch away and back does not lose it.
//!
//! Nothing here draws and nothing here names a colour.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use ubiq_proto::files::{DiffBase, FileDiff};
use ubiq_proto::git::{
    GitChangedPath, GitCommit, GitEntry, GitHead, GitPathChange, GitRef, GitRefKind, GitSubmodule,
};

use crate::state::when;

/// The width of the ref sidebar, and of the uncommitted-changes panel on the other side.
pub const SIDEBAR_WIDTH: f32 = 280.0;
pub const CHANGES_WIDTH: f32 = 380.0;
/// How tall the diff under the history is when it is open, and the height of one commit row.
pub const DIFF_HEIGHT: f32 = 320.0;
pub const COMMIT_ROW: f32 = 26.0;
/// Fixed trailing columns on a history row, so refs of different widths cannot shove author, when
/// or the short id around.
pub const AUTHOR_COL: f32 = 150.0;
pub const WHEN_COL: f32 = 78.0;
pub const SHA_COL: f32 = 70.0;
/// How far apart two lanes of the history graph sit, and how wide the lane gutter is.
pub const LANE_PITCH: f32 = 14.0;
pub const LANE_GUTTER: f32 = 72.0;

/// One collapsible group in the sidebar. The order here is the order they are drawn in.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RefSection {
    Local,
    Remotes,
    Tags,
    Stashes,
    Submodules,
}

impl RefSection {
    pub fn all() -> [RefSection; 5] {
        [
            RefSection::Local,
            RefSection::Remotes,
            RefSection::Tags,
            RefSection::Stashes,
            RefSection::Submodules,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            RefSection::Local => "Local branches",
            RefSection::Remotes => "Remotes",
            RefSection::Tags => "Tags",
            RefSection::Stashes => "Stashes",
            RefSection::Submodules => "Submodules",
        }
    }

    /// Index into the fixed 5-slot grouping `GitView::grouped_refs` builds, in `all()`'s own
    /// order.
    pub(crate) fn slot(self) -> usize {
        match self {
            RefSection::Local => 0,
            RefSection::Remotes => 1,
            RefSection::Tags => 2,
            RefSection::Stashes => 3,
            RefSection::Submodules => 4,
        }
    }
}

/// One row in the sidebar: a branch, a remote-tracking branch, a tag, a stash or a submodule.
#[derive(Clone, Debug, PartialEq)]
pub struct RefRow {
    pub section: RefSection,
    pub name: String,
    /// The commit this ref points at, abbreviated — what a double-click uses to find it in the
    /// history.
    pub target: String,
    /// Commits either side of this ref's upstream. Absent, never zero, when there is none.
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    /// This is what HEAD points at. At most one row answers true.
    pub current: bool,
}

impl RefRow {
    pub fn new(section: RefSection, name: &str) -> Self {
        Self {
            section,
            name: name.to_string(),
            target: String::new(),
            ahead: None,
            behind: None,
            current: false,
        }
    }

    pub fn at(mut self, target: &str) -> Self {
        self.target = target.to_string();
        self
    }

    pub fn tracking(mut self, ahead: u32, behind: u32) -> Self {
        self.ahead = (ahead > 0).then_some(ahead);
        self.behind = (behind > 0).then_some(behind);
        self
    }

    pub fn current(mut self) -> Self {
        self.current = true;
        self
    }
}

/// One row of a slash-split ref tree: a folder for a shared prefix, or a leaf that is a real ref.
#[derive(Clone, Debug, PartialEq)]
pub struct RefTreeRow {
    pub depth: usize,
    /// The last path segment, which is what the row prints.
    pub label: String,
    pub kind: RefTreeKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RefTreeKind {
    /// A prefix that has children. `path` is the joined prefix the folder shuts on.
    Folder {
        path: String,
        open: bool,
        leaves: usize,
    },
    Leaf {
        index: usize,
        /// This ref's name is also a prefix of other refs, so a twisty sits on the row.
        has_children: bool,
        open: bool,
    },
}

/// Fold local branches and remotes on `/`, so `feature/things` nests under `feature` and
/// `origin/feature/things` nests under `origin`, then `feature`.
pub fn ref_tree(rows: &[(usize, &RefRow)], shut: &HashSet<String>) -> Vec<RefTreeRow> {
    #[derive(Default)]
    struct Node {
        children: std::collections::BTreeMap<String, Node>,
        leaf: Option<usize>,
    }

    fn insert(node: &mut Node, parts: &[&str], index: usize) {
        match parts {
            [] => {}
            [last] => {
                node.children.entry((*last).to_string()).or_default().leaf = Some(index);
            }
            [head, rest @ ..] => {
                insert(
                    node.children.entry((*head).to_string()).or_default(),
                    rest,
                    index,
                );
            }
        }
    }

    fn leaf_count(node: &Node) -> usize {
        let here = usize::from(node.leaf.is_some());
        here + node.children.values().map(leaf_count).sum::<usize>()
    }

    fn flatten(
        node: &Node,
        prefix: &str,
        depth: usize,
        shut: &HashSet<String>,
        out: &mut Vec<RefTreeRow>,
    ) {
        for (segment, child) in &node.children {
            let path = if prefix.is_empty() {
                segment.clone()
            } else {
                format!("{prefix}/{segment}")
            };
            let has_children = !child.children.is_empty();
            if has_children && child.leaf.is_none() {
                let open = !shut.contains(&path);
                out.push(RefTreeRow {
                    depth,
                    label: segment.clone(),
                    kind: RefTreeKind::Folder {
                        path: path.clone(),
                        open,
                        leaves: leaf_count(child),
                    },
                });
                if open {
                    flatten(child, &path, depth + 1, shut, out);
                }
            } else if has_children {
                let open = !shut.contains(&path);
                out.push(RefTreeRow {
                    depth,
                    label: segment.clone(),
                    kind: RefTreeKind::Leaf {
                        index: child.leaf.expect("has_children && leaf"),
                        has_children: true,
                        open,
                    },
                });
                if open {
                    flatten(child, &path, depth + 1, shut, out);
                }
            } else if let Some(index) = child.leaf {
                out.push(RefTreeRow {
                    depth,
                    label: segment.clone(),
                    kind: RefTreeKind::Leaf {
                        index,
                        has_children: false,
                        open: true,
                    },
                });
            }
        }
    }

    let mut root = Node::default();
    for (index, row) in rows {
        let parts: Vec<&str> = row
            .name
            .split('/')
            .filter(|part| !part.is_empty())
            .collect();
        if parts.is_empty() {
            continue;
        }
        insert(&mut root, &parts, *index);
    }
    let mut out = Vec::new();
    flatten(&root, "", 0, shut, &mut out);
    out
}

/// The sidebar's rows: Local, Remotes, Tags and Stashes from the refs reply, Submodules from the
/// overview — a submodule is a repository and not a ref, so it never rides on [`GitRef`].
pub fn ref_rows(refs: &[GitRef], submodules: &[GitSubmodule]) -> Vec<RefRow> {
    let mut rows: Vec<RefRow> = refs
        .iter()
        .map(|r| RefRow {
            section: match r.kind {
                GitRefKind::Local => RefSection::Local,
                GitRefKind::Remote => RefSection::Remotes,
                GitRefKind::Tag => RefSection::Tags,
                GitRefKind::Stash => RefSection::Stashes,
            },
            name: r.name.clone(),
            target: r.target.clone(),
            ahead: r.ahead,
            behind: r.behind,
            current: r.current,
        })
        .collect();
    rows.extend(submodule_rows(submodules));
    rows
}

/// The Submodules section on its own, for the overview's refresh — see [`ref_rows`], which builds
/// the same rows for the initial reply.
pub fn submodule_rows(submodules: &[GitSubmodule]) -> Vec<RefRow> {
    submodules
        .iter()
        .map(|sm| RefRow {
            section: RefSection::Submodules,
            name: sm.rel_path.clone(),
            target: String::new(),
            ahead: None,
            behind: None,
            current: false,
        })
        .collect()
}

/// One commit in the history.
///
/// `lane` is which column of the graph the commit's dot sits in, and `merges` are the lanes that
/// join it from the right — enough to rebuild the connectors without the interface inventing a
/// topology of its own.
#[derive(Clone, Debug, PartialEq)]
pub struct CommitRow {
    pub id: String,
    pub short_id: String,
    pub summary: String,
    pub author: String,
    /// How long ago, already worded. The interface prints what it is given rather than doing
    /// arithmetic on a clock it has no offset for.
    pub when: String,
    pub lane: usize,
    pub merges: Vec<usize>,
    /// The commit's parents, crossed the bus as ids — the smallest thing a lane algorithm can
    /// match a child to its parent's lane with. `graph_cells` uses them to tell where a lane
    /// begins and ends.
    pub parents: Vec<String>,
    /// Branch and tag names pointing at this commit, for the decorations.
    pub refs: Vec<String>,
    /// Whether the signed-in user is its author, which is what the history's one filter asks.
    pub mine: bool,
}

/// The history's rows, newest first. `lane` and `merges` are carried straight through from the
/// host's [`GitCommit`], and so are `parents` — the host's lane allocator computed the real
/// topology, so this is a projection, not a computation.
pub fn commit_rows(commits: &[GitCommit]) -> Vec<CommitRow> {
    let now = Utc::now();
    commits
        .iter()
        .map(|c| CommitRow {
            id: c.id.clone(),
            short_id: c.short_id.clone(),
            summary: c.summary.clone(),
            author: c.author.name.clone(),
            when: DateTime::<Utc>::from_timestamp(c.author.time, 0)
                .map(|then| when::relative(then, now))
                .unwrap_or_default(),
            lane: c.lane,
            merges: c.merges.clone(),
            parents: c.parents.clone(),
            refs: c.refs.clone(),
            mine: c.mine,
        })
        .collect()
}

/// How far a commit's own column and the lanes it merges from reach — the widest lane the graph's
/// gutter may have to carry. A merge's extra-parent lanes are claimed or opened by the host and
/// can sit past every commit's own column, so the gutter is sized to them too.
fn graph_lanes(commit: &CommitRow) -> usize {
    let widest_merge = commit.merges.iter().copied().max().unwrap_or(0);
    commit.lane.max(widest_merge) + 1
}

/// One row cell of the history graph, rebuilt from the host's lane data the way the lanes fell:
///
/// - `above` — the lanes that have a live line entering this row from the one above;
/// - `below` — the lanes that keep a live line leaving this row toward the one below;
/// - `joins` — the lanes above whose line **ends** at this row: they were waiting for this
///   commit's id and converge into its dot. An empty `joins` is a commit no loaded line points
///   at — a branch tip.
///
/// Everything is derived, never invented: the columns are the host's `lane`/`merges`, the
/// endpoints are the same parents the host matched on, so two windows computing this from the
/// same log agree with each other and with what the host drew.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphCell {
    pub above: Vec<usize>,
    pub below: Vec<usize>,
    pub joins: Vec<usize>,
}

/// One `GraphCell` per commit, over the loaded history newest first, in one pass.
///
/// The walk mirrors the host's allocator with the finite facts it already sent: a lane waits for
/// whichever parent id was claimed onto it, and is freed when the commit it waits for is reached;
/// a commit claims its own `lane` for its first parent, and each merge lane holds the extra
/// parent that flanked it. The one thing this does not carry is lane continuity across pages —
/// that stays the host's `lanes_for` cache. Here the oldest loaded commit's open lanes simply
/// trail off, and `extend_commits` reruns the walk over the whole log once the next page lands.
pub fn graph_cells(commits: &[CommitRow]) -> Vec<GraphCell> {
    let mut lanes: Vec<Option<&str>> = Vec::new();
    let mut cells = Vec::with_capacity(commits.len());
    for commit in commits {
        let mut above = Vec::new();
        let mut joins = Vec::new();
        for (i, waiting) in lanes.iter().enumerate() {
            if let Some(id) = waiting {
                above.push(i);
                if *id == commit.id {
                    joins.push(i);
                }
            }
        }
        for waiting in lanes.iter_mut() {
            if waiting.is_some_and(|id| id == commit.id) {
                *waiting = None;
            }
        }
        let dot = commit.lane;
        while lanes.len() <= dot {
            lanes.push(None);
        }
        lanes[dot] = commit.parents.first().map(String::as_str);
        for (j, &merge) in commit.merges.iter().enumerate() {
            if let Some(parent) = commit.parents.get(j + 1) {
                while lanes.len() <= merge {
                    lanes.push(None);
                }
                lanes[merge] = Some(parent.as_str());
            }
        }
        let below: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter_map(|(i, waiting)| waiting.is_some().then_some(i))
            .collect();
        cells.push(GraphCell {
            above,
            below,
            joins,
        });
    }
    cells
}

/// One commit's search haystack — its summary, author and short id, lowercased and joined behind
/// a separator no field or typed query realistically contains, so `visible_commits`'s one
/// `.contains` check does what three separate `.to_lowercase()`-then-`.contains` calls did.
/// See `GitView::search_cache`, which builds this once per commit rather than once per frame.
fn commit_haystack(commit: &CommitRow) -> String {
    format!(
        "{}\u{0}{}\u{0}{}\u{0}{}",
        commit.summary.to_lowercase(),
        commit.author.to_lowercase(),
        commit.short_id.to_lowercase(),
        commit.id.to_lowercase()
    )
}

/// One collapsible group in the changes panel. Conflicted draws first when it is not empty;
/// Staged and Unstaged are always on screen, even at zero.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ChangeSection {
    Conflicted,
    Staged,
    Unstaged,
}

impl ChangeSection {
    pub fn label(self) -> &'static str {
        match self {
            ChangeSection::Conflicted => "Conflicted",
            ChangeSection::Staged => "Staged",
            ChangeSection::Unstaged => "Unstaged",
        }
    }
}

/// Which of the three change lists a row is in. A path that is both staged and modified appears
/// in Staged and Unstaged; a conflicted path is only ever in Conflicted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Conflicted,
    Staged,
    Unstaged,
}

impl Side {
    /// What a row from this list is compared against, when the screen is not already comparing
    /// two commits.
    pub fn base(self) -> DiffBase {
        match self {
            Side::Conflicted => DiffBase::Head,
            Side::Staged => DiffBase::Staged,
            Side::Unstaged => DiffBase::Index,
        }
    }

    pub fn label(self) -> &'static str {
        self.section().label()
    }

    pub fn section(self) -> ChangeSection {
        match self {
            Side::Conflicted => ChangeSection::Conflicted,
            Side::Staged => ChangeSection::Staged,
            Side::Unstaged => ChangeSection::Unstaged,
        }
    }
}

/// A write or refresh the screen has sent and not yet heard back from. Drawn as a status word
/// until a working-tree reply, or a failed write, clears it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitPending {
    Stage,
    Unstage,
    StageAll,
    UnstageAll,
    Commit,
    FetchAll,
    Pull,
    Push,
    Refresh,
    Log,
    Compare,
}

impl GitPending {
    pub fn label(self) -> &'static str {
        match self {
            GitPending::Stage | GitPending::StageAll => "Staging\u{2026}",
            GitPending::Unstage | GitPending::UnstageAll => "Unstaging\u{2026}",
            GitPending::Commit => "Committing\u{2026}",
            GitPending::FetchAll => "Fetching\u{2026}",
            GitPending::Pull => "Pulling\u{2026}",
            GitPending::Push => "Pushing\u{2026}",
            GitPending::Refresh => "Refreshing\u{2026}",
            GitPending::Log => "Loading history\u{2026}",
            GitPending::Compare => "Comparing\u{2026}",
        }
    }
}

/// The menu a right-click on the Git screen raised, until it is dismissed or another menu takes
/// its place.
#[derive(Clone, Debug)]
pub struct GitMenu {
    /// Which menu this is, so a dismiss aimed at a menu that has already been replaced does
    /// nothing. See [`GitView::close_menu`].
    pub epoch: u64,
    pub kind: GitMenuKind,
    pub x: f32,
    pub y: f32,
}

/// Which row a Git context menu opened on.
#[derive(Clone, Debug)]
pub enum GitMenuKind {
    /// A changed path. `changes.rs` raises this from a file row.
    Change {
        path: String,
        side: Side,
    },
    Commit {
        index: usize,
    },
    Ref {
        index: usize,
    },
}

/// One row of a Git context menu, matched by index the way the explorer's is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GitMenuEntry {
    pub action: GitAction,
    pub enabled: bool,
}

impl GitMenuEntry {
    pub fn label(self) -> &'static str {
        self.action.label()
    }

    pub fn is_separator(self) -> bool {
        self.action == GitAction::Separator
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GitAction {
    Stage,
    Unstage,
    Discard,
    Open,
    CopyPath,
    CopySha,
    CherryPick,
    Revert,
    Reset,
    Checkout,
    Merge,
    Delete,
    CopyName,
    Separator,
}

impl GitAction {
    pub fn label(self) -> &'static str {
        match self {
            GitAction::Stage => "Stage",
            GitAction::Unstage => "Unstage",
            GitAction::Discard => "Discard",
            GitAction::Open => "Open",
            GitAction::CopyPath => "Copy path",
            GitAction::CopySha => "Copy SHA",
            GitAction::CherryPick => "Cherry-pick",
            GitAction::Revert => "Revert",
            GitAction::Reset => "Reset",
            GitAction::Checkout => "Checkout",
            GitAction::Merge => "Merge",
            GitAction::Delete => "Delete",
            GitAction::CopyName => "Copy name",
            GitAction::Separator => "",
        }
    }
}

impl GitMenu {
    /// What this click offers, in the order the menu draws it. `stageable`/`unstageable` are
    /// frozen from the working tree as of open, so a pick's index still names the row that was
    /// drawn even if the path has since moved.
    pub fn entries(&self, stageable: bool, unstageable: bool) -> Vec<GitMenuEntry> {
        let row = |action, enabled| GitMenuEntry { action, enabled };
        let dead = |action| row(action, false);
        match &self.kind {
            GitMenuKind::Change { .. } => vec![
                row(GitAction::Stage, stageable),
                row(GitAction::Unstage, unstageable),
                row(GitAction::Separator, false),
                dead(GitAction::Discard),
                dead(GitAction::Open),
                row(GitAction::CopyPath, true),
            ],
            GitMenuKind::Commit { .. } => vec![
                row(GitAction::CopySha, true),
                row(GitAction::Separator, false),
                dead(GitAction::CherryPick),
                dead(GitAction::Revert),
                dead(GitAction::Reset),
            ],
            GitMenuKind::Ref { .. } => vec![
                dead(GitAction::Checkout),
                dead(GitAction::Merge),
                dead(GitAction::Delete),
                row(GitAction::Separator, false),
                row(GitAction::CopyName, true),
            ],
        }
    }
}

/// What `HEAD` reads as in one short label: a branch name, a detached short id, or the branch an
/// unborn repository is on. Stated once so the status bar and the explorer's nested-repository
/// rows cannot word the same fact differently. Empty for a repository the host could not read.
pub fn head_label(head: &GitHead) -> String {
    match head {
        GitHead::Branch(name) => name.clone(),
        GitHead::Detached { short_id } => format!("detached {short_id}"),
        GitHead::Unborn(name) => name.clone(),
    }
}

/// The letter a change draws, on the same one-letter discipline as the explorer's badges.
pub fn change_letter(change: &GitPathChange) -> &'static str {
    match change {
        GitPathChange::Added => "A",
        GitPathChange::Modified => "M",
        GitPathChange::Deleted => "D",
        GitPathChange::Renamed { .. } => "R",
        GitPathChange::TypeChange => "T",
        GitPathChange::Untracked => "U",
    }
}

/// The paths whose index differs from HEAD: what a commit would carry.
pub fn staged(entries: &[GitEntry]) -> Vec<&GitEntry> {
    entries
        .iter()
        .filter(|entry| !entry.conflicted && entry.index.is_some())
        .collect()
}

/// The paths whose worktree differs from the index. Useful for the commit count's counterpart —
/// what a commit would leave behind.
pub fn unstaged(entries: &[GitEntry]) -> Vec<&GitEntry> {
    entries
        .iter()
        .filter(|entry| !entry.conflicted && entry.worktree.is_some())
        .collect()
}

/// The paths with unmerged stages. Drawn first and on their own, because nothing else on the
/// screen can be acted on until they are gone.
pub fn conflicted(entries: &[GitEntry]) -> Vec<&GitEntry> {
    entries.iter().filter(|entry| entry.conflicted).collect()
}

/// Whether `+` can stage this path: it has worktree content and is not conflicted.
pub fn can_stage(entry: &GitEntry) -> bool {
    !entry.conflicted && entry.worktree.is_some()
}

/// Whether `-` can unstage this path: it has index content and is not conflicted.
pub fn can_unstage(entry: &GitEntry) -> bool {
    !entry.conflicted && entry.index.is_some()
}

/// [`conflicted`] first, then staged, then unstaged. A conflicted path is only in conflicted; a
/// path with both index and worktree content is in staged and unstaged.
///
/// Indices rather than references, because the list that draws them is virtual: it holds the
/// grouping across frames and looks each row's entry up when it builds the rows on screen.
pub struct ChangeGroups {
    pub conflicted: Vec<usize>,
    pub staged: Vec<usize>,
    pub unstaged: Vec<usize>,
}

pub fn group_changes(entries: &[GitEntry]) -> ChangeGroups {
    let mut groups = ChangeGroups {
        conflicted: Vec::new(),
        staged: Vec::new(),
        unstaged: Vec::new(),
    };
    for (index, entry) in entries.iter().enumerate() {
        if entry.conflicted {
            groups.conflicted.push(index);
        } else {
            if entry.index.is_some() {
                groups.staged.push(index);
            }
            if entry.worktree.is_some() {
                groups.unstaged.push(index);
            }
        }
    }
    groups
}

/// The Git screen's view of one project's repository.
pub struct GitView {
    /// The sections the user has shut. Absent means open, so a screen that has never been touched
    /// shows everything it has.
    shut: HashSet<RefSection>,
    /// Slash-prefix folders the user has shut in the local and remote trees. Absent means open.
    shut_folders: HashSet<(RefSection, String)>,
    /// Which sidebar row is selected, as an index into `refs`.
    pub selected_ref: Option<usize>,

    /// What was typed into the history's search field.
    pub search: String,
    /// Restrict the history to this ref's walk. `None` is HEAD, the walk a project opens on.
    pub branch_filter: Option<String>,
    /// The history's one filter: only commits the signed-in user wrote.
    pub mine_only: bool,
    /// Which commit is selected, as an index into `commits`. **`None` is the uncommitted row**,
    /// which is a real selection and the one the screen opens on — not "nothing selected".
    pub selected_commit: Option<usize>,
    /// The other end of a two-commit comparison, as an index into `commits`. Set together with
    /// `selected_commit`; the changes panel lists the paths between them while both are present.
    pub compare_commit: Option<usize>,

    /// The change-list sections the user has shut. Absent means open, same as `shut`.
    shut_changes: HashSet<ChangeSection>,

    /// The changed path the diff under the history is about, and which list it was picked from.
    pub selected_path: Option<(Side, String)>,
    /// What that path is being compared against, which is the list's own answer. Kept so a reply
    /// for the other base — a diff tab asking about the same path — is not mistaken for this one.
    pub base: DiffBase,
    /// The hunks the host computed for that path. Absent while the read is in flight, and thrown
    /// away when the selection moves so a stale comparison is never drawn under a new name.
    pub diff: Option<FileDiff>,

    /// Side by side rather than one column. The two arrangements the diff renderer has; there is
    /// no wrap and no whitespace switch here, because the hunks the host sends have neither.
    pub split: bool,
    /// Whether the diff is on screen at all. The history is the screen's subject and the diff is
    /// what it is being read for, so the pane shuts rather than the history shrinking.
    pub diff_open: bool,

    /// What is typed in the commit box, and whether it would amend. Kept across a project switch
    /// for the reason every other composer's draft is.
    pub message: String,
    pub amend: bool,

    /// The last write that failed, drawn under the commit box until the next successful working
    /// tree arrives.
    pub last_error: Option<String>,
    /// A write or refresh still in flight. Cleared by a working-tree reply or a failed write.
    pub pending: Option<GitPending>,

    /// The sidebar's rows, from the host's refs and the overview's submodules.
    pub refs: Vec<RefRow>,
    /// The history, oldest page first, newest commit first within it. Only ever replaced or
    /// extended through [`GitView::set_commits`]/[`GitView::extend_commits`], which keep
    /// `search_haystacks`, `graph` and `lane_count` beside it — see those for why.
    pub commits: Vec<CommitRow>,
    /// Each commit's lowercased search haystack (see [`commit_haystack`]), in step with
    /// `commits` — built once when a page lands rather than cached against `commits.len()`,
    /// so a same-length in-place replacement (a rebase that does not change the commit count)
    /// can never serve a stale haystack.
    search_haystacks: Vec<String>,
    /// One [`GraphCell`] per commit, in step with `commits`. Kept beside them the same way and
    /// for the same reason as `search_haystacks`: edges are a projection of the host's lane data,
    /// so they are rebuilt when a page lands, not recomputed every frame.
    pub graph: Vec<GraphCell>,
    /// How many lanes wide the graph is, kept beside `commits` the same way and for the same
    /// reason as `search_haystacks`.
    lane_count: usize,
    /// The commit after the last one in `commits` — what the next page's request would start
    /// from. `None` before the first page has landed.
    pub log_cursor: Option<String>,
    /// Whether the last page had no `next_cursor`: the history has nothing more to page in.
    pub log_done: bool,
    /// The cursor of the `ProjectGitLog` request currently in flight, i.e. the one whose
    /// `GitLogPage` reply the view is waiting on. `None` means no reply is outstanding.
    ///
    /// This is what tells a stale reply from the current one: two requests can carry the same
    /// `cursor` (two first-page requests both ask with `None`, for instance a refresh racing the
    /// project's initial load), so the echoed `cursor` field alone cannot tell them apart. Sending
    /// a new request overwrites this unconditionally, so only the most recently sent request's
    /// reply matches — every other reply, whenever it lands, is stale and is discarded rather than
    /// applied.
    pub log_inflight: Option<Option<String>>,

    /// The two revs a range comparison asked for, and the paths that differ between them.
    /// `range_from` absent with `range_to` set is a commit against the empty tree.
    pub range_from: Option<String>,
    pub range_to: Option<String>,
    pub range_files: Vec<GitChangedPath>,
    /// A `ProjectGitChanged` request is in flight. Stale `GitChanged` replies are discarded
    /// unless their `from`/`to` still match these.
    pub range_inflight: bool,

    /// The right-click menu, while it is down. Absent is no menu of this screen's.
    pub menu: Option<GitMenu>,
    /// Stamped onto every menu that opens, so a dismiss can say which menu it was aimed at.
    menu_epoch: u64,
}

impl GitView {
    /// The screen a project opens on: everything showing, the uncommitted row selected, the diff
    /// pane open and unified.
    pub fn new(refs: Vec<RefRow>, commits: Vec<CommitRow>) -> Self {
        let mut view = Self {
            shut: HashSet::new(),
            shut_folders: HashSet::new(),
            selected_ref: refs.iter().position(|row| row.current),
            search: String::new(),
            branch_filter: None,
            mine_only: false,
            selected_commit: None,
            compare_commit: None,
            shut_changes: HashSet::new(),
            selected_path: None,
            base: DiffBase::Head,
            diff: None,
            split: false,
            diff_open: true,
            message: String::new(),
            amend: false,
            last_error: None,
            pending: None,
            refs,
            commits: Vec::new(),
            search_haystacks: Vec::new(),
            graph: Vec::new(),
            lane_count: 0,
            log_cursor: None,
            log_done: false,
            log_inflight: None,
            range_from: None,
            range_to: None,
            range_files: Vec::new(),
            range_inflight: false,
            menu: None,
            menu_epoch: 0,
        };
        view.set_commits(commits);
        view
    }

    pub fn is_open(&self, section: RefSection) -> bool {
        !self.shut.contains(&section)
    }

    pub fn toggle_section(&mut self, section: RefSection) {
        if !self.shut.remove(&section) {
            self.shut.insert(section);
        }
    }

    pub fn is_change_open(&self, section: ChangeSection) -> bool {
        !self.shut_changes.contains(&section)
    }

    pub fn toggle_change_section(&mut self, section: ChangeSection) {
        if !self.shut_changes.remove(&section) {
            self.shut_changes.insert(section);
        }
    }

    /// A word for whatever the screen is still waiting on: a write, a history page, or a range
    /// comparison. The write wins when more than one is in flight.
    pub fn in_progress(&self) -> Option<&'static str> {
        self.pending
            .map(GitPending::label)
            .or(self
                .log_inflight
                .is_some()
                .then_some("Loading history\u{2026}"))
            .or(self.range_inflight.then_some("Comparing\u{2026}"))
    }

    /// Whether this history row is one end of the current selection: the selected commit, or the
    /// other end of a two-commit comparison.
    pub fn commit_selected(&self, index: usize) -> bool {
        self.selected_commit == Some(index) || self.compare_commit == Some(index)
    }

    /// The two ids a range comparison would ask for, older then newer. History is newest first, so
    /// the larger index is the older commit. Absent unless both ends are set and still in the list.
    pub fn compare_ids(&self) -> Option<(String, String)> {
        let a = self.selected_commit?;
        let b = self.compare_commit?;
        let (older, newer) = if a > b { (a, b) } else { (b, a) };
        Some((
            self.commits.get(older)?.id.clone(),
            self.commits.get(newer)?.id.clone(),
        ))
    }

    /// Cmd/ctrl-click on a commit: toggle it as the other end of a comparison. Returns whether a
    /// pair is now set.
    ///
    /// Uncommitted cannot be compared, so a click while that row is selected is a normal select.
    /// Clicking the already-selected commit, or the compare commit again, clears the pair only.
    pub fn toggle_compare(&mut self, index: usize) -> bool {
        if self.selected_commit.is_none() {
            self.selected_commit = Some(index);
            self.compare_commit = None;
            return false;
        }
        if self.selected_commit == Some(index) || self.compare_commit == Some(index) {
            self.compare_commit = None;
            return false;
        }
        self.compare_commit = Some(index);
        true
    }

    /// Raise the menu at the pointer.
    pub fn open_menu(&mut self, kind: GitMenuKind, x: f32, y: f32) {
        self.menu_epoch = self.menu_epoch.wrapping_add(1);
        self.menu = Some(GitMenu {
            epoch: self.menu_epoch,
            kind,
            x,
            y,
        });
    }

    /// Take the menu away, if the menu that is up is still the one being dismissed.
    pub fn close_menu(&mut self, epoch: u64) {
        if self.menu.as_ref().is_some_and(|menu| menu.epoch == epoch) {
            self.menu = None;
        }
    }

    pub fn toggle_folder(&mut self, section: RefSection, path: &str) {
        let key = (section, path.to_string());
        if !self.shut_folders.remove(&key) {
            self.shut_folders.insert(key);
        }
    }

    /// The local-branch or remote tree for one section, with shut folders collapsed.
    pub fn ref_tree(&self, section: RefSection) -> Vec<RefTreeRow> {
        let rows = self.rows(section);
        let shut: HashSet<String> = self
            .shut_folders
            .iter()
            .filter(|(held, _)| *held == section)
            .map(|(_, path)| path.clone())
            .collect();
        ref_tree(&rows, &shut)
    }

    /// The commit in the loaded history this ref points at, if that page has landed.
    pub fn commit_index_for_ref(&self, index: usize) -> Option<usize> {
        let row = self.refs.get(index)?;
        self.commits.iter().position(|commit| {
            (!row.target.is_empty()
                && (commit.short_id == row.target || commit.id.starts_with(&row.target)))
                || commit.refs.iter().any(|name| name == &row.name)
        })
    }

    /// The rows in one section, with the index each is selected by.
    pub fn rows(&self, section: RefSection) -> Vec<(usize, &RefRow)> {
        self.refs
            .iter()
            .enumerate()
            .filter(|(_, row)| row.section == section)
            .collect()
    }

    /// How many rows a section has, which is what its heading reports while it is shut.
    pub fn count(&self, section: RefSection) -> usize {
        self.refs
            .iter()
            .filter(|row| row.section == section)
            .count()
    }

    /// Every ref grouped into its section, in one pass over `refs` — what `rows` and `count`
    /// filtered the whole vector for separately, once per section, five passes a frame across the
    /// sidebar's five sections.
    pub fn grouped_refs(&self) -> [Vec<(usize, &RefRow)>; 5] {
        let mut groups: [Vec<(usize, &RefRow)>; 5] =
            [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for (index, row) in self.refs.iter().enumerate() {
            groups[row.section.slot()].push((index, row));
        }
        groups
    }

    /// The commits the history is drawing, with the index each is selected by.
    ///
    /// The search matches the summary, the author and the abbreviated id, case-insensitively —
    /// the three things a user has in hand when they go looking for a commit.
    pub fn visible_commits(&self) -> Vec<(usize, &CommitRow)> {
        let needle = self.search.trim().to_lowercase();
        self.commits
            .iter()
            .zip(self.search_haystacks.iter())
            .enumerate()
            .filter(|(_, (commit, _))| !self.mine_only || commit.mine)
            .filter(|(_, (_, haystack))| needle.is_empty() || haystack.contains(&needle))
            .map(|(index, (commit, _))| (index, commit))
            .collect()
    }

    /// How many lanes wide the graph is. Zero commits is zero lanes, not one. Kept beside
    /// `commits` by `set_commits`/`extend_commits` rather than maxed over every commit every
    /// frame just to size the gutter.
    pub fn lanes(&self) -> usize {
        self.lane_count
    }

    /// Replace the whole history — a `GitLogPage` reply for the first page, or a refresh
    /// restarting it. Computes the search haystack, the graph's cells and the lane count here,
    /// once, so a same-length in-place replacement (a rebase, say) never serves any of them
    /// stale.
    pub fn set_commits(&mut self, commits: Vec<CommitRow>) {
        self.search_haystacks = commits.iter().map(commit_haystack).collect();
        self.graph = graph_cells(&commits);
        self.lane_count = commits.iter().map(graph_lanes).max().unwrap_or(0);
        self.commits = commits;
    }

    /// Append the next page onto the history, extending the haystack, the graph's cells and the
    /// lane count with it — a `GitLogPage` reply whose `cursor` is not the first page's.
    ///
    /// The graph is the one thing spliced rather than extended: a lane that trailed off the old
    /// oldest page may close on the new one, so `graph_cells` reruns over the whole log for the
    /// cells of every row, not just the appended page's.
    pub fn extend_commits(&mut self, commits: Vec<CommitRow>) {
        self.search_haystacks
            .extend(commits.iter().map(commit_haystack));
        self.commits.extend(commits);
        self.graph = graph_cells(&self.commits);
        self.lane_count = self
            .lane_count
            .max(self.commits.iter().map(graph_lanes).max().unwrap_or(0));
    }

    /// The rows the history is drawing, each with the graph cell that one commit draws, in the
    /// order the list shows them.
    ///
    /// The search matches the summary, the author and the abbreviated id, case-insensitively —
    /// the three things a user has in hand when they go looking for a commit. The cells are the
    /// loaded history's, not the filtered list's: a search hides rows, it does not relayout the
    /// graph, so the lines keep the shape the host's lanes gave them.
    pub fn history(&self) -> Vec<(usize, &CommitRow, &GraphCell)> {
        let needle = self.search.trim().to_lowercase();
        self.commits
            .iter()
            .zip(self.search_haystacks.iter())
            .zip(self.graph.iter())
            .enumerate()
            .filter(|(_, ((commit, _), _))| !self.mine_only || commit.mine)
            .filter(|(_, ((_, haystack), _))| needle.is_empty() || haystack.contains(&needle))
            .map(|(index, ((commit, _), cell))| (index, commit, cell))
            .collect()
    }

    /// Whether the history is showing everything it has.
    pub fn filtered(&self) -> bool {
        self.mine_only || !self.search.trim().is_empty() || self.branch_filter.is_some()
    }

    /// Show the whole history again. One control clears every filter, so a history emptied by a
    /// filter is always one click from being full.
    pub fn clear_filters(&mut self) {
        self.mine_only = false;
        self.search.clear();
        self.branch_filter = None;
    }

    /// Point the diff pane at a changed path. Returns whether the selection moved, which is what
    /// tells the caller a comparison has to be asked for. The same path from Staged and from
    /// Unstaged are two questions: the bases differ.
    pub fn select_path(&mut self, side: Side, path: &str) -> bool {
        if self
            .selected_path
            .as_ref()
            .is_some_and(|(held_side, held)| *held_side == side && held == path)
        {
            return false;
        }
        self.selected_path = Some((side, path.to_string()));
        self.base = if self.range_from.is_some() {
            DiffBase::Commits
        } else {
            side.base()
        };
        self.diff = None;
        true
    }

    /// The path the diff pane is about, whatever list it came from.
    pub fn path(&self) -> Option<&str> {
        self.selected_path.as_ref().map(|(_, path)| path.as_str())
    }

    /// Drop a selection whose path the working tree no longer has anything to say about. A diff
    /// left under a name that has gone clean is a comparison of nothing. While a range comparison
    /// is active, the path has to still be in `range_files`.
    pub fn settle(&mut self, entries: &[GitEntry]) {
        let Some((_, path)) = &self.selected_path else {
            return;
        };
        let path = path.clone();
        let keep = if self.range_from.is_some() {
            self.range_files.iter().any(|file| file.rel_path == path)
        } else {
            entries.iter().any(|entry| entry.rel_path == path)
        };
        if !keep {
            self.selected_path = None;
            self.diff = None;
        }
    }
}

impl Default for GitView {
    fn default() -> Self {
        Self::new(Vec::new(), Vec::new())
    }
}
