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

/// Whether the tree follows the active editor. Three states, cycled by one button: off, a single
/// reveal, and locked to every change of active editor. Never written down — a follow that
/// survived a restart would move the tree before the user had asked anything.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Follow {
    #[default]
    Off,
    /// Reveal what is open now, once.
    Once,
    /// Reveal every file that becomes the active editor.
    Locked,
}

impl Follow {
    /// The next state the button's click asks for.
    pub fn next(self) -> Self {
        match self {
            Follow::Off => Follow::Once,
            Follow::Once => Follow::Locked,
            Follow::Locked => Follow::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Follow::Off => "Follow the active editor",
            Follow::Once => "Revealed once — click to lock",
            Follow::Locked => "Following the active editor",
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
    /// Remove the row the keyboard is on — Backspace on macOS, Delete elsewhere.
    Delete,
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
    /// Delete on a row: the window raises its own confirmation. The tree removes nothing itself,
    /// because a removal is the host's and a question has to be answered first.
    Remove { path: String, is_dir: bool },
}

/// What a right-click on a row (or on the empty panel) offers.
///
/// Every row here is answered. Creating, renaming, copying and removing a path all go out as one
/// `EditProjectPath` with the op on it, so the menu is a list of gestures rather than a list of
/// message names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExplorerAction {
    Open,
    OpenDiff,
    CopyPath,
    CopyFullPath,
    /// The row as a `ubiq://` destination, which is what a link in a document wants.
    CopyLink,
    OpenInSystem,
    OpenInWeb,
    Refresh,
    NewFile,
    NewFolder,
    /// Remember the row, so a later Paste knows what to copy. Nothing crosses the bus for it.
    Copy,
    /// Copy whatever [`ExplorerAction::Copy`] remembered into this folder.
    Paste,
    /// Copy the row beside itself, under a free name.
    Duplicate,
    Rename,
    Delete,
    CollapseAll,
    /// The line between two groups. It is an action so that it occupies a slot in the list the
    /// pick indexes into: a separator drawn but not counted would shift every row below it.
    Separator,
}

impl ExplorerAction {
    /// What the row says. A separator says nothing: it is drawn as a hairline.
    pub fn label(self) -> &'static str {
        match self {
            ExplorerAction::Open => "Open",
            ExplorerAction::OpenDiff => "Open diff vs HEAD",
            ExplorerAction::CopyPath => "Copy path",
            ExplorerAction::CopyFullPath => "Copy full path",
            ExplorerAction::CopyLink => "Copy link",
            ExplorerAction::OpenInSystem => open_in_system_label(),
            ExplorerAction::OpenInWeb => "Open in Web",
            ExplorerAction::Refresh => "Refresh",
            ExplorerAction::NewFile => "New file",
            ExplorerAction::NewFolder => "New folder",
            ExplorerAction::Copy => "Copy",
            ExplorerAction::Paste => "Paste",
            ExplorerAction::Duplicate => "Duplicate",
            ExplorerAction::Rename => "Rename",
            ExplorerAction::Delete => "Delete",
            ExplorerAction::CollapseAll => "Collapse all",
            ExplorerAction::Separator => "",
        }
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
    /// Whether the row can be picked. Only ever false for [`ExplorerAction::Paste`] with nothing
    /// remembered: the row stays visible, because a Paste that vanished would read as a menu that
    /// does not offer one at all.
    pub enabled: bool,
}

impl ExplorerEntry {
    pub fn label(self) -> &'static str {
        self.action.label()
    }

    /// Whether this entry is the line between two groups rather than a row.
    pub fn is_separator(self) -> bool {
        self.action == ExplorerAction::Separator
    }
}

/// The menu a right-click raised, until it is dismissed or another menu takes its place.
#[derive(Clone, Debug)]
pub struct ExplorerMenu {
    /// Which menu this is, so a dismiss aimed at a menu that has already been replaced does
    /// nothing. See [`ExplorerState::close_menu`].
    pub epoch: u64,
    /// The row that was clicked. Absent is a click on the empty panel, which still has a menu —
    /// new file, new folder, collapse all — because that is where those actions live when no row
    /// is under the pointer.
    pub path: Option<String>,
    pub is_dir: bool,
    pub readable: bool,
    pub expanded: bool,
    /// Whether anything was remembered by a Copy when this menu opened. Held here so `entries()`
    /// stays a pure function of the remembered menu, which is what keeps the pick — an index into
    /// this list — pointing at the row that was drawn.
    pub can_paste: bool,
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
            self.can_paste,
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
    /// The project's name, which is the tree's own first row. Empty until the window has the
    /// project's record in hand — the constructor has no name to give it.
    pub root_name: String,
    /// Whether that row is open. It is the one handle that collapses the whole project, so it
    /// starts open and stays drawn even while a filter is typed.
    pub root_expanded: bool,
    pub selected: Option<String>,
    /// Whether the host's ceiling cut the project's top level short.
    pub truncated: bool,
    /// Whether the root has ever been listed. Without it an empty tree and a tree whose first
    /// listing has not arrived look identical, and every remembered folder would be dropped as
    /// gone in the frame before the host answers.
    root_listed: bool,
    pub view: ExplorerView,
    /// Whether the tree reveals the active editor's file. Not persisted.
    pub follow: Follow,
    /// Which row the keyboard is on. A path rather than an index: rows come and go as folders open
    /// and the filter narrows, and an index would be pointing at a different row afterwards.
    cursor: Option<String>,
    pub menu: Option<ExplorerMenu>,
    /// Stamped onto every menu that opens, so a dismiss can say which menu it was aimed at. A
    /// right-click on a second row raises a new menu *and* fires the old one's outside-click for
    /// the same event, and an unconditional close would shut the menu that was just opened.
    menu_epoch: u64,
    /// What a Copy remembered, until a Paste uses it or an edit takes the path away. One path
    /// rather than a list: the menu copies the row that was clicked.
    pub copied: Option<String>,
    /// The folder a drag is currently over, which is the only answer the user gets before letting
    /// go. Empty is the project's root — the scroll container itself is a drop target.
    pub drop_onto: Option<String>,
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
    root_name: String,
    root_expanded: bool,
    pub view: ExplorerView,
    cursor: Option<String>,
    selected: Option<String>,
}

mod filter;
mod keys;
mod menu;
mod rows;
mod tree;

pub use menu::menu_entries;
