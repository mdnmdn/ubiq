//! The file tree the explorer draws, and the git state each row carries.
//!
//! **The tree is what the host has said so far, never a guess.** A folder is listed one level at a
//! time, on the expand that asks for it, so a project with a `node_modules` in it costs one row
//! rather than a walk. A folder therefore has three states a flattened row has to carry apart —
//! shut, open, and waiting for its listing — and it has a twisty before anything is known about
//! it, because a listing says what is inside a folder and not whether there is anything.
//!
//! **Tree and list are the same set, arranged twice.** The tree is the folders the user walked
//! into; the list is every match the host has already named, flat, each with the folder it came
//! from. Which one is on screen is the user's choice. A filter finds rather than prunes: every
//! folder already listed is walked while one is typed, and a folder with nothing matching under it
//! drops out instead of drawing as empty. The listings themselves are filled in the background
//! when the project opens, so a search reads a cache rather than waiting on the host — except the
//! walk's skip set, which is how `node_modules` stays one row. Filtering a large cache is done
//! off the frame, after a short debounce, so typing a letter does not stall the window.
//!
//! **Git state is an `Option`, and `None` is not "clean".** Until a working-tree map has arrived,
//! every row is unmarked because nothing has been read. Once a map is here, a row not in it is
//! clean, which draws the same and means a different thing — the status bar's branch is how a
//! repository is known. An untracked or ignored directory is the exception: git does not look
//! inside, so every child inherits that status. Colour, a leading mark and a badge are how a
//! status is drawn.
//!
//! Everything here is a tree, a merge and a flatten: no frame, no bus, no path on disk. That is
//! what lets `tests/explorer.rs` assert the restore rules without a window.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ubiq_proto::files::{DirEntry, DirListing, EntryKind, WALK_SKIP};
use ubiq_proto::git::{GitEntry, GitMark, GitRollup};

/// How a file stands against the index. The explorer tints the name and shows a single-letter
/// badge from this, never from wording alone.
///
/// Every variant marks a row. A file with nothing to say about it carries no status at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GitStatus {
    Modified,
    Untracked,
    Conflict,
    Staged,
    Ignored,
}

impl GitStatus {
    /// The badge shown at the end of the row.
    pub fn badge(self) -> &'static str {
        match self {
            GitStatus::Modified => "M",
            GitStatus::Untracked => "U",
            GitStatus::Conflict => "!",
            GitStatus::Staged => "S",
            GitStatus::Ignored => "ignored",
        }
    }

    /// The interface's single status, projected from the pair the host sent.
    pub fn from_mark(mark: GitMark) -> Self {
        match mark {
            GitMark::Modified => GitStatus::Modified,
            GitMark::Untracked => GitStatus::Untracked,
            GitMark::Conflict => GitStatus::Conflict,
            GitMark::Staged => GitStatus::Staged,
            GitMark::Ignored => GitStatus::Ignored,
        }
    }

    fn rank(self) -> u8 {
        match self {
            GitStatus::Ignored => 1,
            GitStatus::Staged => 2,
            GitStatus::Untracked => 3,
            GitStatus::Modified => 4,
            GitStatus::Conflict => 5,
        }
    }
}

#[derive(Clone, Debug)]
pub enum NodeKind {
    Dir {
        /// What the host said is inside. Empty until it has said anything.
        ///
        /// An `Arc` so a filter walk can hold a snapshot without cloning the tree on the frame.
        children: Arc<Vec<FileNode>>,
        expanded: bool,
        /// Whether the host has ever listed this folder. Not the same as having children: a folder
        /// that has been listed and is empty is a different thing from one nobody has asked about.
        listed: bool,
        /// A listing has been asked for and has not arrived.
        loading: bool,
        /// Whether the host's entry ceiling cut the listing short, so the row can say so rather
        /// than draw a folder as smaller than it is.
        truncated: bool,
    },
    File,
}

#[derive(Clone, Debug)]
pub struct FileNode {
    pub name: String,
    /// Project-relative, as every path the interface holds is.
    pub path: String,
    pub kind: NodeKind,
    /// What version control says about the file, when anything does.
    pub git: Option<GitStatus>,
    /// Whether the host will open or list it. Something it will not follow — a symlink out of the
    /// project, a socket, a device — is drawn faint rather than hidden, because a tree with rows
    /// missing is a tree that lies.
    pub readable: bool,
}

impl FileNode {
    /// The node one listed entry becomes: shut, and unlisted until the host says what is inside it.
    fn from_entry(entry: DirEntry) -> Self {
        let kind = match entry.kind {
            EntryKind::Dir => NodeKind::Dir {
                children: Arc::new(Vec::new()),
                expanded: false,
                listed: false,
                loading: false,
                truncated: false,
            },
            // Something the host will not follow has nothing to expand into, so it draws as a leaf
            // and is marked unreadable rather than given a twisty that leads nowhere.
            EntryKind::File | EntryKind::Other => NodeKind::File,
        };
        Self {
            name: entry.name,
            path: entry.rel_path,
            kind,
            git: None,
            readable: entry.kind != EntryKind::Other,
        }
    }

    /// The same node, told what the host now says about it. Everything known below it survives,
    /// which is what keeps an expanded tree expanded across a re-listing.
    fn refreshed(mut self, entry: DirEntry) -> Self {
        self.path = entry.rel_path;
        self.readable = entry.kind != EntryKind::Other;
        self
    }

    /// Whether a listed entry is the same kind of thing this node already is. A name that is a
    /// file where it was a folder is a different thing on disk, and nothing about the old node is
    /// worth keeping.
    fn same_kind(&self, entry: &DirEntry) -> bool {
        matches!(self.kind, NodeKind::Dir { .. }) == (entry.kind == EntryKind::Dir)
    }

    fn is_dir(&self) -> bool {
        matches!(self.kind, NodeKind::Dir { .. })
    }
}

/// How the same set is arranged. The user's choice, kept while the project is open.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ExplorerView {
    #[default]
    Tree,
    List,
}

impl ExplorerView {
    pub fn label(self) -> &'static str {
        match self {
            ExplorerView::Tree => "Tree view",
            ExplorerView::List => "List view",
        }
    }
}

/// A key the explorer answers to, told apart from the ones it does not.
///
/// The keystrokes themselves are `ui::explorer`'s: what a platform calls confirm is not something
/// the tree's rules should have an opinion about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExplorerKey {
    Up,
    Down,
    /// Shut the folder the keyboard is on, or step out to the one holding it.
    Left,
    /// Open the folder the keyboard is on, or step into it.
    Right,
    /// Open the file the keyboard is on, or toggle the folder.
    Enter,
    /// Shift+Enter: open the file permanently (opposite of temp preview).
    ShiftEnter,
    Dismiss,
}

/// What a key press turned out to mean for the window holding the tree.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ExplorerPressed {
    /// Nothing here answers this key. Whoever else wants it may have it — which is how `left` and
    /// `right` go back to being the filter field's caret keys in the flat list.
    Ignored,
    /// The cursor moved, a folder opened, or a folder shut.
    Moved,
    /// Enter on a file: open it.
    Open { path: String },
    /// A folder was opened that the host has never listed.
    Listing { path: String },
    /// The context menu went away.
    Dismissed,
    /// Escape on an empty menu with a filter: the field should go back to blank.
    ClearFilter,
}

/// What a right-click on a row (or on the empty panel) offers.
///
/// New file, new folder, rename and delete are drawn and not yet answered: nothing on the bus
/// creates or removes a path, and a menu that hid those rows would have nowhere to put them when
/// the host grows the family.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExplorerAction {
    Open,
    OpenDiff,
    CopyPath,
    CopyFullPath,
    OpenInSystem,
    OpenInWeb,
    Refresh,
    NewFile,
    NewFolder,
    Rename,
    Delete,
    Toggle,
    CollapseAll,
}

impl ExplorerAction {
    /// What the row says. `expanded` is only read for [`ExplorerAction::Toggle`].
    pub fn label(self, expanded: bool) -> &'static str {
        match self {
            ExplorerAction::Open => "Open",
            ExplorerAction::OpenDiff => "Open diff vs HEAD",
            ExplorerAction::CopyPath => "Copy path",
            ExplorerAction::CopyFullPath => "Copy full path",
            ExplorerAction::OpenInSystem => open_in_system_label(),
            ExplorerAction::OpenInWeb => "Open in Web",
            ExplorerAction::Refresh => "Refresh",
            ExplorerAction::NewFile => "New file",
            ExplorerAction::NewFolder => "New folder",
            ExplorerAction::Rename => "Rename",
            ExplorerAction::Delete => "Delete",
            ExplorerAction::Toggle if expanded => "Collapse",
            ExplorerAction::Toggle => "Expand",
            ExplorerAction::CollapseAll => "Collapse all",
        }
    }

    /// Whether the window can do it today. The four that create or remove a path wait on the host.
    pub fn ready(self) -> bool {
        !matches!(
            self,
            ExplorerAction::NewFile
                | ExplorerAction::NewFolder
                | ExplorerAction::Rename
                | ExplorerAction::Delete
        )
    }
}

fn open_in_system_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Open in Finder"
    } else if cfg!(target_os = "windows") {
        "Open in Explorer"
    } else {
        "Open in File Manager"
    }
}

/// One entry in the menu a right-click raises.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExplorerEntry {
    pub action: ExplorerAction,
    pub expanded: bool,
}

impl ExplorerEntry {
    pub fn label(self) -> &'static str {
        self.action.label(self.expanded)
    }

    pub fn ready(self) -> bool {
        self.action.ready()
    }
}

/// The menu a right-click raised, until it is dismissed or another menu takes its place.
#[derive(Clone, Debug)]
pub struct ExplorerMenu {
    /// The row that was clicked. Absent is a click on the empty panel, which still has a menu —
    /// new file, new folder, collapse all — because that is where those actions live when no row
    /// is under the pointer.
    pub path: Option<String>,
    pub is_dir: bool,
    pub readable: bool,
    pub expanded: bool,
    pub x: f32,
    pub y: f32,
}

impl ExplorerMenu {
    /// What this click offers, in the order the menu draws it.
    pub fn entries(&self) -> Vec<ExplorerEntry> {
        menu_entries(
            self.path.as_deref(),
            self.is_dir,
            self.readable,
            self.expanded,
        )
    }
}

/// One visible line, already arranged for whichever view is on screen.
#[derive(Clone, Debug)]
pub struct Row {
    pub name: String,
    pub path: String,
    /// How far in the tree indents it. Always zero in the list, which is what flat means.
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
    /// A listing is on its way. The row says so rather than looking like an empty folder.
    pub loading: bool,
    /// The host's ceiling cut this folder's listing short.
    pub truncated: bool,
    pub git: Option<GitStatus>,
    pub readable: bool,
    /// Whether the keyboard is on this row. Selection is the open file; the cursor is only where
    /// the next key lands, and the two are drawn differently because they mean different things.
    pub on_cursor: bool,
    /// What the row says at its far end: which folder it is in, in the list.
    pub trailing: String,
}

/// What flipping a folder turned out to mean.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Toggle {
    /// Now open, and nothing is known about what is inside it: the host has to be asked.
    Listing,
    /// Now open or shut, and nothing needs asking.
    Done,
    /// No such folder in the tree.
    Missing,
}

/// Whether a remembered folder could be opened yet.
enum Reach {
    /// It is there and is now open. `needs_listing` is whether the host still has to say what is
    /// inside it.
    Opened { needs_listing: bool },
    /// A folder on the way to it has not been listed, so nothing can be said about it yet.
    Waiting,
    /// A folder that *has* been listed does not hold it. It was renamed or deleted while nobody
    /// was looking.
    Gone,
}

pub struct ExplorerState {
    pub root: Arc<Vec<FileNode>>,
    pub selected: Option<String>,
    /// Whether the host's ceiling cut the project's top level short.
    pub truncated: bool,
    /// Whether the root has ever been listed. Without it an empty tree and a tree whose first
    /// listing has not arrived look identical, and every remembered folder would be dropped as
    /// gone in the frame before the host answers.
    root_listed: bool,
    pub view: ExplorerView,
    /// Which row the keyboard is on. A path rather than an index: rows come and go as folders open
    /// and the filter narrows, and an index would be pointing at a different row afterwards.
    cursor: Option<String>,
    pub menu: Option<ExplorerMenu>,
    /// Set for the click that opened the menu, so that click's own outside-dismiss cannot close it
    /// before it has been drawn.
    pub menu_held: bool,
    /// Folders the background cache has already asked the host about. Without it a listing that
    /// failed would be asked for again as the next reply landed, and a listing still in flight
    /// would be asked twice.
    cache_asked: HashSet<String>,
    /// The last background filter result. Drawn instead of walking the tree on the frame.
    filter_hits: Option<FilterHits>,
    /// Incremented every time a filter job starts or is cancelled, so a slow walk cannot land on
    /// a query the user has already left.
    filter_job: u64,
    /// Project-relative path → status, from the last working-tree map that was not stale. Files
    /// come from the map's entries; folders from its rollups.
    git_marks: HashMap<String, GitStatus>,
    /// Paths the host reported as untracked or ignored directories. Expanding one inherits that
    /// status onto every child: git does not look inside, and a child not in the map is not clean.
    git_inherit: HashSet<String>,
    /// Whether a working-tree map has arrived. Without it every row is unmarked because nothing
    /// has been read; with it a row not in `git_marks` is clean.
    git_known: bool,
    git_generation: u64,
}

/// What a background filter walk returns. `needle` is trimmed and lowercased, matching `rows`.
#[derive(Clone, Debug)]
struct FilterHits {
    needle: String,
    view: ExplorerView,
    rows: Vec<Row>,
}

/// Enough of the tree to walk off the frame. No menus, no cache-asked set, no hits of its own.
pub struct FilterSnap {
    root: Arc<Vec<FileNode>>,
    pub view: ExplorerView,
    cursor: Option<String>,
    selected: Option<String>,
}

impl ExplorerState {
    /// A tree that knows nothing yet, which is every tree until the host answers.
    pub fn empty() -> Self {
        Self {
            root: Arc::new(Vec::new()),
            selected: None,
            truncated: false,
            root_listed: false,
            view: ExplorerView::Tree,
            cursor: None,
            menu: None,
            menu_held: false,
            cache_asked: HashSet::new(),
            filter_hits: None,
            filter_job: 0,
            git_marks: HashMap::new(),
            git_inherit: HashSet::new(),
            git_known: false,
            git_generation: 0,
        }
    }

    /// Whether the project's top level has arrived.
    pub fn is_listed(&self) -> bool {
        self.root_listed
    }

    /// Whether a folder's children have already arrived, which is what says a re-listing would
    /// fold into the tree rather than be thrown away. The empty path is the root.
    pub fn is_folder_listed(&self, path: &str) -> bool {
        if path.is_empty() {
            return self.root_listed;
        }
        matches!(
            node_of(&self.root, path),
            Some(FileNode {
                kind: NodeKind::Dir { listed: true, .. },
                ..
            })
        )
    }

    /// The status the repository reports for an exact path, if it has been read at all.
    pub fn git_status(&self, path: &str) -> Option<GitStatus> {
        if self.git_known {
            self.git_marks.get(path).copied()
        } else {
            None
        }
    }

    /// The rows the explorer draws, in the view it is in.
    ///
    /// The filter is the window's rather than the tree's, because one field drives whichever
    /// project is on screen and a copy per tree would disagree with it on every switch.
    ///
    /// **Filtering is finding, not pruning.** A non-empty filter walks every folder the host has
    /// already named, and a folder with nothing matching under it drops out instead of drawing as
    /// empty — otherwise the answer to a search is a screen of empty folders. Only what the host
    /// has already named can match, which is the honest answer for a tree that is listed a level
    /// at a time rather than a promise about folders nobody has opened.
    pub fn rows(&self, filter: &str) -> Vec<Row> {
        let needle = filter.trim().to_lowercase();
        match self.view {
            ExplorerView::Tree => {
                let mut out = Vec::new();
                self.tree_rows(&self.root, 0, &needle, &mut out);
                out
            }
            ExplorerView::List => self.list_rows(&needle),
        }
    }

    /// Walk a snapshot off the frame. Same rules as [`ExplorerState::rows`].
    pub fn rows_from_snap(snap: FilterSnap, filter: &str) -> Vec<Row> {
        let mut tree = ExplorerState::empty();
        tree.root = snap.root;
        tree.view = snap.view;
        tree.cursor = snap.cursor;
        tree.selected = snap.selected;
        tree.root_listed = true;
        tree.rows(filter)
    }

    /// What the panel draws. An empty filter walks only open folders. A non-empty one borrows the
    /// last background result, and never clones or walks the cache on the frame.
    pub fn drawn_rows(&self, filter: &str) -> std::borrow::Cow<'_, [Row]> {
        if filter.trim().is_empty() {
            return std::borrow::Cow::Owned(self.rows(""));
        }
        match self.hits_ref(filter) {
            Some(rows) => std::borrow::Cow::Borrowed(rows),
            None => std::borrow::Cow::Borrowed(&[]),
        }
    }

    /// What the keyboard walks. Hits when they match the filter; a fresh walk otherwise, so
    /// `tests/explorer.rs` can press keys without a background job.
    fn visible_rows(&self, filter: &str) -> Vec<Row> {
        if filter.trim().is_empty() {
            return self.rows("");
        }
        self.hits_for(filter).unwrap_or_else(|| self.rows(filter))
    }

    fn hits_ref(&self, filter: &str) -> Option<&[Row]> {
        let hits = self.filter_hits.as_ref()?;
        if hits.view != self.view {
            return None;
        }
        if hits.needle != filter.trim().to_lowercase() {
            return None;
        }
        Some(&hits.rows)
    }

    fn hits_for(&self, filter: &str) -> Option<Vec<Row>> {
        self.hits_ref(filter).map(|rows| rows.to_vec())
    }

    /// A snapshot the background thread can walk. Cloning the `Arc` is the point: the frame must
    /// not copy the tree to start a search.
    pub fn filter_snap(&self) -> FilterSnap {
        FilterSnap {
            root: Arc::clone(&self.root),
            view: self.view,
            cursor: self.cursor.clone(),
            selected: self.selected.clone(),
        }
    }

    /// Start a background filter walk. Answers the job id the result has to carry back.
    pub fn begin_filter(&mut self) -> u64 {
        self.filter_job = self.filter_job.wrapping_add(1);
        self.filter_job
    }

    /// Land a background walk, answering whether it was still the one that was asked for.
    pub fn apply_hits(
        &mut self,
        job: u64,
        filter: String,
        view: ExplorerView,
        rows: Vec<Row>,
    ) -> bool {
        if job != self.filter_job || self.view != view {
            return false;
        }
        self.filter_hits = Some(FilterHits {
            needle: filter.trim().to_lowercase(),
            view,
            rows,
        });
        self.reanchor_hits();
        self.sync_hit_cursors();
        true
    }

    /// Drop hits and cancel any walk still in flight. Clearing the field is immediate.
    pub fn clear_filter(&mut self) {
        self.filter_hits = None;
        self.filter_job = self.filter_job.wrapping_add(1);
    }

    fn reanchor_hits(&mut self) {
        let Some(hits) = &self.filter_hits else {
            return;
        };
        let held = self
            .cursor
            .as_deref()
            .is_some_and(|path| hits.rows.iter().any(|row| row.path == path));
        if !held {
            self.cursor = hits.rows.first().map(|row| row.path.clone());
        }
    }

    fn sync_hit_cursors(&mut self) {
        let cursor = self.cursor.clone();
        if let Some(hits) = &mut self.filter_hits {
            for row in &mut hits.rows {
                row.on_cursor = cursor.as_deref() == Some(row.path.as_str());
            }
        }
    }

    /// The tree: folders the user walked into, and the matches under them.
    fn tree_rows(&self, nodes: &[FileNode], depth: usize, needle: &str, out: &mut Vec<Row>) {
        if !needle.is_empty() {
            self.tree_rows_filtered(nodes, depth, needle, out);
            return;
        }
        for node in nodes {
            if node.is_dir() {
                let (expanded, loading, truncated) = dir_flags(node);
                out.push(self.row(node, depth, expanded, loading, truncated, String::new()));
                if expanded && let NodeKind::Dir { children, .. } = &node.kind {
                    self.tree_rows(children, depth + 1, needle, out);
                }
                continue;
            }
            out.push(self.row(node, depth, false, false, false, String::new()));
        }
    }

    /// One walk, not a subtree test per folder: the old `subtree_matches` was quadratic, which is
    /// what a letter in the field was paying for.
    fn tree_rows_filtered(
        &self,
        nodes: &[FileNode],
        depth: usize,
        needle: &str,
        out: &mut Vec<Row>,
    ) {
        for node in nodes {
            if node.is_dir() {
                let mut kids = Vec::new();
                if let NodeKind::Dir { children, .. } = &node.kind {
                    self.tree_rows_filtered(children, depth + 1, needle, &mut kids);
                }
                let self_match = node.path.to_lowercase().contains(needle);
                if self_match || !kids.is_empty() {
                    let (_, loading, truncated) = dir_flags(node);
                    out.push(self.row(node, depth, true, loading, truncated, String::new()));
                    out.append(&mut kids);
                }
                continue;
            }
            if node.path.to_lowercase().contains(needle) {
                out.push(self.row(node, depth, false, false, false, String::new()));
            }
        }
    }

    /// The list: every match the host has already named, flat, each said to be in the folder it
    /// is in. Sorted by name without case, because a flat list is read by name — the folder is
    /// the answer to "which one is this", not the thing the eye is scanning.
    fn list_rows(&self, needle: &str) -> Vec<Row> {
        let mut flat = Vec::new();
        collect_listed(&self.root, needle, &mut flat);
        flat.sort_by_key(|node| node.name.to_lowercase());
        flat.into_iter()
            .map(|node| {
                let (expanded, loading, truncated) = dir_flags(node);
                self.row(node, 0, expanded, loading, truncated, parent_of(&node.path))
            })
            .collect()
    }

    fn row(
        &self,
        node: &FileNode,
        depth: usize,
        expanded: bool,
        loading: bool,
        truncated: bool,
        trailing: String,
    ) -> Row {
        Row {
            name: node.name.clone(),
            path: node.path.clone(),
            depth,
            is_dir: node.is_dir(),
            expanded,
            loading,
            truncated,
            git: node.git,
            readable: node.readable,
            on_cursor: self.cursor.as_deref() == Some(node.path.as_str()),
            trailing,
        }
    }

    /// Flip a folder open or shut, answering whether its children have to be asked for.
    pub fn toggle(&mut self, path: &str) -> Toggle {
        let Some(node) = node_mut(cow(&mut self.root), path) else {
            return Toggle::Missing;
        };
        let NodeKind::Dir {
            expanded,
            listed,
            loading,
            ..
        } = &mut node.kind
        else {
            return Toggle::Done;
        };

        *expanded = !*expanded;
        if *expanded && !*listed && !*loading {
            Toggle::Listing
        } else {
            Toggle::Done
        }
    }

    /// Note that a listing is on its way, so the row can say so.
    pub fn set_loading(&mut self, path: &str, loading: bool) {
        if let Some(node) = node_mut(cow(&mut self.root), path)
            && let NodeKind::Dir { loading: flag, .. } = &mut node.kind
        {
            *flag = loading;
        }
    }

    /// Put one directory's listing into the tree, keeping everything known below it.
    ///
    /// Entries are matched **by name**: one already there keeps its children and its expanded
    /// flag, one that has gone is dropped with its subtree, and a new one arrives shut and
    /// unlisted. That is what makes a re-listing — a restore, or one day a filesystem watch —
    /// idempotent rather than destructive.
    ///
    /// The host's order is kept as it came. It sorts directories first and names without case
    /// precisely so that two windows agree, and re-sorting here would put that back in doubt.
    ///
    /// Answers false when the listing names a folder the tree does not hold, which is what a
    /// listing for a folder collapsed away while it was in flight looks like.
    pub fn merge(&mut self, listing: DirListing) -> bool {
        let ok = if listing.rel_path.is_empty() {
            merge_children(cow(&mut self.root), listing.entries);
            self.root_listed = true;
            self.truncated = listing.truncated;
            true
        } else if let Some(node) = node_mut(cow(&mut self.root), &listing.rel_path) {
            let NodeKind::Dir {
                children,
                listed,
                loading,
                truncated,
                ..
            } = &mut node.kind
            else {
                return false;
            };
            merge_children(cow(children), listing.entries);
            *listed = true;
            *loading = false;
            *truncated = listing.truncated;
            true
        } else {
            false
        };
        if ok {
            self.paint_git();
        }
        ok
    }

    /// Apply a working-tree map. A reply older than what is already held is discarded.
    pub fn apply_git(
        &mut self,
        generation: u64,
        entries: &[GitEntry],
        rollups: &[GitRollup],
    ) -> bool {
        if self.git_known && generation < self.git_generation {
            return false;
        }
        self.git_generation = generation;
        self.git_known = true;
        self.git_marks.clear();
        self.git_inherit.clear();
        for entry in entries {
            if let Some(mark) = entry.mark() {
                let status = GitStatus::from_mark(mark);
                let path = entry.rel_path.trim_end_matches('/').to_string();
                self.git_marks.insert(path.clone(), status);
                if matches!(status, GitStatus::Untracked | GitStatus::Ignored) {
                    self.git_inherit.insert(path);
                }
            }
        }
        for rollup in rollups {
            let mark = GitStatus::from_mark(rollup.mark);
            self.git_marks
                .entry(rollup.rel_path.clone())
                .and_modify(|held| {
                    if mark.rank() > held.rank() {
                        *held = mark;
                    }
                })
                .or_insert(mark);
        }
        self.paint_git();
        true
    }

    /// Forget every mark. A project with no repository, or a corrupt one, must not keep the last
    /// good answer on screen.
    pub fn clear_git(&mut self) {
        self.git_known = false;
        self.git_generation = 0;
        self.git_marks.clear();
        self.git_inherit.clear();
        self.paint_git();
    }

    fn paint_git(&mut self) {
        let known = self.git_known;
        paint_nodes(
            cow(&mut self.root),
            known,
            &self.git_marks,
            &self.git_inherit,
            None,
        );
    }

    /// Which folders are open, shallowest first — what the window writes down for the project.
    pub fn expanded(&self) -> Vec<String> {
        let mut out = Vec::new();
        collect_expanded(&self.root, &mut out);
        out
    }

    /// Open the remembered folders that have become reachable, answering the ones whose children
    /// have to be asked for now.
    ///
    /// A deep folder cannot be opened before its parents have been listed, so `wanted` is worked
    /// through again after every listing and shrinks as it goes: a path that opens leaves the
    /// list, one whose listed parent does not hold it is dropped as gone, and one still out of
    /// reach waits for the next answer. That terminates because each listing either resolves a
    /// path or removes it.
    ///
    /// A folder returned here is marked as loading, so a second pass over the same `wanted` cannot
    /// ask for it twice.
    pub fn reopen(&mut self, wanted: &mut Vec<String>) -> Vec<String> {
        // Shallowest first, so a parent is opened and asked about before its child is looked for
        // at all.
        wanted.sort_by_key(|path| path.matches('/').count());

        let mut asking = Vec::new();
        let root_listed = self.root_listed;
        let root = cow(&mut self.root);
        wanted.retain(|path| match reach(root, root_listed, path) {
            Reach::Opened { needs_listing } => {
                if needs_listing {
                    asking.push(path.clone());
                }
                false
            }
            Reach::Waiting => true,
            Reach::Gone => false,
        });
        asking
    }

    /// Shut every folder, keeping what the host has already said about them. Collapsing is not
    /// forgetting: reopening one draws immediately rather than asking again.
    pub fn collapse_all(&mut self) {
        collapse_in(cow(&mut self.root));
        self.menu = None;
        self.menu_held = false;
    }

    pub fn set_view(&mut self, view: ExplorerView, filter: &str) {
        self.view = view;
        if filter.trim().is_empty() {
            self.reanchor(filter);
        } else {
            // Hits belong to the other arrangement; a background walk will replace them.
            self.filter_hits = None;
            self.filter_job = self.filter_job.wrapping_add(1);
        }
    }

    pub fn reanchor(&mut self, filter: &str) {
        let rows = self.visible_rows(filter);
        let held = self
            .cursor
            .as_deref()
            .is_some_and(|path| rows.iter().any(|row| row.path == path));
        if !held {
            self.cursor = rows.first().map(|row| row.path.clone());
        }
        self.sync_hit_cursors();
    }

    /// Folders the background cache still cannot see into, and which it is allowed to ask about.
    ///
    /// A name in [`WALK_SKIP`] is left alone: the host would not descend into it on a deep walk,
    /// and asking for it explicitly would list `node_modules` in full. Folders already asked about
    /// are skipped too, so a failed listing is not asked twice.
    pub fn unlisted_for_cache(&self) -> Vec<String> {
        let mut out = Vec::new();
        collect_cache(&self.root, &self.cache_asked, &mut out);
        out
    }

    /// Note that the cache has asked about these folders. They are marked loading so an expand
    /// while the answer is in flight does not ask again; a shut folder does not draw as spinning.
    pub fn begin_cache(&mut self, paths: &[String]) {
        for path in paths {
            self.cache_asked.insert(path.clone());
            self.set_loading(path, true);
        }
    }

    /// Which row the keyboard is on, and where it is in what is drawn.
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// Where the cursor sits in the rows on screen, which is what a scroll has to be told.
    pub fn cursor_index(&self, filter: &str) -> Option<usize> {
        self.index_in(&self.visible_rows(filter))
    }

    /// The keyboard follows the mouse: an arrow after a click carries on from the row that was
    /// clicked, not from wherever the cursor was left.
    pub fn set_cursor(&mut self, path: &str) {
        self.cursor = Some(path.to_string());
        self.sync_hit_cursors();
    }

    /// What a click on a row means: a folder in the tree opens, a file is the thing to open, and
    /// a folder in the list is only where the cursor lands — there is no depth to walk into.
    pub fn click(&mut self, path: &str) -> ExplorerPressed {
        self.set_cursor(path);
        let (readable, is_dir) = match node_of(&self.root, path) {
            Some(node) => (node.readable, node.is_dir()),
            None => return ExplorerPressed::Ignored,
        };
        if !readable {
            return ExplorerPressed::Ignored;
        }
        if is_dir {
            if self.view == ExplorerView::Tree {
                return self.toggle_result(path);
            }
            return ExplorerPressed::Moved;
        }
        ExplorerPressed::Open {
            path: path.to_string(),
        }
    }

    fn toggle_result(&mut self, path: &str) -> ExplorerPressed {
        match self.toggle(path) {
            Toggle::Listing => ExplorerPressed::Listing {
                path: path.to_string(),
            },
            Toggle::Done => ExplorerPressed::Moved,
            Toggle::Missing => ExplorerPressed::Ignored,
        }
    }

    // ── the keyboard ────────────────────────────────────────────────

    /// What a key means here, and what is left for whoever else wants it.
    ///
    /// **Every rule is in this one function**, so the explorer behaves the same however the key
    /// arrived — and so `tests/explorer.rs` can press keys without a window.
    pub fn press(&mut self, key: ExplorerKey, filter: &str) -> ExplorerPressed {
        let pressed = match key {
            ExplorerKey::Dismiss => self.dismiss(filter),
            ExplorerKey::Up => self.step(-1, filter),
            ExplorerKey::Down => self.step(1, filter),
            ExplorerKey::Left => self.step_out(filter),
            ExplorerKey::Right => self.step_in(filter),
            ExplorerKey::Enter => self.enter(filter),
            ExplorerKey::ShiftEnter => self.enter(filter),
        };
        self.sync_hit_cursors();
        pressed
    }

    fn index_in(&self, rows: &[Row]) -> Option<usize> {
        let cursor = self.cursor.as_deref()?;
        rows.iter().position(|row| row.path == cursor)
    }

    /// One row up or down, stopping at the ends rather than wrapping — a list that wraps loses the
    /// user the moment they hold the key down.
    fn step(&mut self, delta: isize, filter: &str) -> ExplorerPressed {
        let rows = self.visible_rows(filter);
        if rows.is_empty() {
            return ExplorerPressed::Ignored;
        }
        let next = match self.index_in(&rows) {
            Some(at) => (at as isize + delta).clamp(0, rows.len() as isize - 1) as usize,
            None if delta > 0 => 0,
            None => rows.len() - 1,
        };
        self.cursor = Some(rows[next].path.clone());
        ExplorerPressed::Moved
    }

    /// Open the folder the cursor is on, or — where it is already open — step into it.
    fn step_in(&mut self, filter: &str) -> ExplorerPressed {
        let rows = self.visible_rows(filter);
        let Some(at) = self.index_in(&rows) else {
            return ExplorerPressed::Ignored;
        };
        let row = rows[at].clone();
        if self.view != ExplorerView::Tree || !row.is_dir {
            return ExplorerPressed::Ignored;
        }

        if self.needs_listing(&row.path) {
            return self.toggle_result(&row.path);
        }

        if !self.is_expanded(&row.path) && filter.trim().is_empty() {
            return self.toggle_result(&row.path);
        }

        match rows.get(at + 1).filter(|next| next.depth > row.depth) {
            Some(child) => {
                self.cursor = Some(child.path.clone());
                ExplorerPressed::Moved
            }
            None => ExplorerPressed::Ignored,
        }
    }

    /// Shut the folder the cursor is on, or step out to the folder holding it.
    fn step_out(&mut self, filter: &str) -> ExplorerPressed {
        let rows = self.visible_rows(filter);
        let Some(at) = self.index_in(&rows) else {
            return ExplorerPressed::Ignored;
        };
        let row = rows[at].clone();
        if self.view != ExplorerView::Tree {
            return ExplorerPressed::Ignored;
        }

        // While a filter is typed every folder is drawn open, so shutting one would change nothing
        // on screen. Stepping out still means something, and that is what it does.
        if row.is_dir && self.is_expanded(&row.path) && filter.trim().is_empty() {
            return self.toggle_result(&row.path);
        }

        let depth = row.depth;
        match rows[..at].iter().rposition(|above| above.depth < depth) {
            Some(parent) => {
                self.cursor = Some(rows[parent].path.clone());
                ExplorerPressed::Moved
            }
            None => ExplorerPressed::Ignored,
        }
    }

    fn enter(&mut self, filter: &str) -> ExplorerPressed {
        let rows = self.visible_rows(filter);
        let Some(at) = self.index_in(&rows) else {
            return ExplorerPressed::Ignored;
        };
        let row = rows[at].clone();
        if !row.readable {
            return ExplorerPressed::Ignored;
        }
        if row.is_dir {
            if self.view == ExplorerView::Tree {
                return self.toggle_result(&row.path);
            }
            return ExplorerPressed::Moved;
        }
        ExplorerPressed::Open { path: row.path }
    }

    fn dismiss(&mut self, filter: &str) -> ExplorerPressed {
        if self.menu.is_some() {
            self.menu = None;
            self.menu_held = false;
            return ExplorerPressed::Dismissed;
        }
        if !filter.trim().is_empty() {
            return ExplorerPressed::ClearFilter;
        }
        ExplorerPressed::Ignored
    }

    fn is_expanded(&self, path: &str) -> bool {
        match node_of(&self.root, path) {
            Some(node) => matches!(node.kind, NodeKind::Dir { expanded: true, .. }),
            None => false,
        }
    }

    fn needs_listing(&self, path: &str) -> bool {
        match node_of(&self.root, path) {
            Some(node) => matches!(
                node.kind,
                NodeKind::Dir {
                    listed: false,
                    loading: false,
                    ..
                }
            ),
            None => false,
        }
    }

    // ── the menu ────────────────────────────────────────────────────

    /// Raise the menu at the pointer, remembering enough of the row to draw it after the tree
    /// has moved on.
    pub fn open_menu(&mut self, path: Option<&str>, x: f32, y: f32) {
        let (is_dir, readable, expanded) = match path {
            Some(path) => match node_of(&self.root, path) {
                Some(node) => {
                    let (expanded, _, _) = dir_flags(node);
                    (node.is_dir(), node.readable, expanded)
                }
                None => (false, false, false),
            },
            None => (false, true, false),
        };
        if let Some(path) = path {
            self.cursor = Some(path.to_string());
        }
        self.menu = Some(ExplorerMenu {
            path: path.map(str::to_string),
            is_dir,
            readable,
            expanded,
            x,
            y,
        });
        self.menu_held = true;
    }

    pub fn close_menu(&mut self) {
        if self.menu_held {
            self.menu_held = false;
            return;
        }
        self.menu = None;
    }
}

fn cow(nodes: &mut Arc<Vec<FileNode>>) -> &mut Vec<FileNode> {
    Arc::make_mut(nodes)
}

fn paint_nodes(
    nodes: &mut [FileNode],
    known: bool,
    marks: &HashMap<String, GitStatus>,
    inherit_from: &HashSet<String>,
    inherited: Option<GitStatus>,
) {
    for node in nodes {
        node.git = if known {
            marks.get(&node.path).copied().or(inherited)
        } else {
            None
        };
        if let NodeKind::Dir { children, .. } = &mut node.kind {
            let next = if inherit_from.contains(&node.path) {
                node.git
            } else {
                inherited
            };
            paint_nodes(cow(children), known, marks, inherit_from, next);
        }
    }
}

fn merge_children(existing: &mut Vec<FileNode>, entries: Vec<DirEntry>) {
    let mut kept: Vec<FileNode> = Vec::with_capacity(entries.len());
    for entry in entries {
        let previous = existing
            .iter()
            .position(|node| node.name == entry.name)
            .map(|at| existing.remove(at));
        kept.push(match previous {
            Some(node) if node.same_kind(&entry) => node.refreshed(entry),
            _ => FileNode::from_entry(entry),
        });
    }
    // Whatever is left was not in the listing: it has gone, and its subtree with it.
    *existing = kept;
}

fn dir_flags(node: &FileNode) -> (bool, bool, bool) {
    match &node.kind {
        NodeKind::Dir {
            expanded,
            loading,
            truncated,
            ..
        } => (*expanded, *loading, *truncated),
        NodeKind::File => (false, false, false),
    }
}

fn walk_skipped(path: &str) -> bool {
    path.split('/').any(|part| WALK_SKIP.contains(&part))
}

fn collect_cache(nodes: &[FileNode], asked: &HashSet<String>, out: &mut Vec<String>) {
    for node in nodes {
        let NodeKind::Dir {
            listed,
            loading,
            children,
            ..
        } = &node.kind
        else {
            continue;
        };
        if *listed {
            collect_cache(children, asked, out);
            continue;
        }
        if !node.readable || *loading || asked.contains(&node.path) || walk_skipped(&node.path) {
            continue;
        }
        out.push(node.path.clone());
    }
}

fn collect_listed<'a>(nodes: &'a [FileNode], needle: &str, out: &mut Vec<&'a FileNode>) {
    for node in nodes {
        if needle.is_empty() || node.path.to_lowercase().contains(needle) {
            out.push(node);
        }
        if let NodeKind::Dir { children, .. } = &node.kind {
            collect_listed(children, needle, out);
        }
    }
}

/// The folder an entry is in, as the list writes it. The project root is `.`, because a blank
/// column would read as missing rather than as "at the top".
fn parent_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((parent, _)) if !parent.is_empty() => parent.to_string(),
        _ => ".".to_string(),
    }
}

/// What a right-click offers for this row — or for the empty panel, when `path` is absent.
pub fn menu_entries(
    path: Option<&str>,
    is_dir: bool,
    readable: bool,
    expanded: bool,
) -> Vec<ExplorerEntry> {
    let entry = |action| ExplorerEntry { action, expanded };
    match path {
        None => vec![
            entry(ExplorerAction::NewFile),
            entry(ExplorerAction::NewFolder),
            entry(ExplorerAction::CollapseAll),
        ],
        Some(_) if is_dir => {
            let mut items = vec![entry(ExplorerAction::Toggle)];
            if readable {
                items.extend([
                    entry(ExplorerAction::NewFile),
                    entry(ExplorerAction::NewFolder),
                ]);
            }
            items.push(entry(ExplorerAction::CopyPath));
            items.push(entry(ExplorerAction::CopyFullPath));
            items.push(entry(ExplorerAction::OpenInSystem));
            items.push(entry(ExplorerAction::OpenInWeb));
            if readable {
                items.push(entry(ExplorerAction::Refresh));
            }
            if readable {
                items.extend([entry(ExplorerAction::Rename), entry(ExplorerAction::Delete)]);
            }
            items
        }
        Some(_) => {
            let mut items = Vec::new();
            if readable {
                items.extend([entry(ExplorerAction::Open), entry(ExplorerAction::OpenDiff)]);
            }
            items.push(entry(ExplorerAction::CopyPath));
            items.push(entry(ExplorerAction::CopyFullPath));
            items.push(entry(ExplorerAction::OpenInSystem));
            items.push(entry(ExplorerAction::OpenInWeb));
            if readable {
                items.extend([entry(ExplorerAction::Rename), entry(ExplorerAction::Delete)]);
            }
            items
        }
    }
}

fn node_of<'a>(nodes: &'a [FileNode], path: &str) -> Option<&'a FileNode> {
    for node in nodes {
        if node.path == path {
            return Some(node);
        }
        if let NodeKind::Dir { children, .. } = &node.kind
            && let Some(found) = node_of(children, path)
        {
            return Some(found);
        }
    }
    None
}

fn node_mut<'a>(nodes: &'a mut [FileNode], path: &str) -> Option<&'a mut FileNode> {
    for node in nodes.iter_mut() {
        if node.path == path {
            return Some(node);
        }
        if let NodeKind::Dir { children, .. } = &mut node.kind
            && let Some(found) = node_mut(cow(children), path)
        {
            return Some(found);
        }
    }
    None
}

fn collect_expanded(nodes: &[FileNode], out: &mut Vec<String>) {
    for node in nodes {
        if let NodeKind::Dir {
            children, expanded, ..
        } = &node.kind
            && *expanded
        {
            out.push(node.path.clone());
            collect_expanded(children, out);
        }
    }
}

fn collapse_in(nodes: &mut [FileNode]) {
    for node in nodes {
        if let NodeKind::Dir {
            children, expanded, ..
        } = &mut node.kind
        {
            *expanded = false;
            collapse_in(cow(children));
        }
    }
}

/// Walk down to `path`, opening it if it is there.
///
/// The two answers that are not "opened" are the whole point: a name a listed folder does not hold
/// is gone, and a name below a folder nobody has listed is simply not knowable yet.
fn reach(nodes: &mut Vec<FileNode>, root_listed: bool, path: &str) -> Reach {
    if path.is_empty() {
        return Reach::Gone;
    }

    let mut nodes = nodes;
    let mut listed = root_listed;
    let mut parts = path.split('/').peekable();

    while let Some(name) = parts.next() {
        let last = parts.peek().is_none();
        let Some(at) = nodes.iter().position(|node| node.name == name) else {
            return if listed { Reach::Gone } else { Reach::Waiting };
        };

        let (child_listed, children) = match &mut nodes[at].kind {
            NodeKind::Dir {
                children,
                expanded,
                listed,
                loading,
                ..
            } => {
                if last {
                    *expanded = true;
                    let needs_listing = !*listed && !*loading;
                    if needs_listing {
                        *loading = true;
                    }
                    return Reach::Opened { needs_listing };
                }
                (*listed, children)
            }
            // A file where a folder was remembered: what is on disk is not what was written down.
            NodeKind::File => return Reach::Gone,
        };

        listed = child_listed;
        nodes = cow(children);
    }

    Reach::Gone
}
