//! The file tree the explorer draws, and the git state each row carries.
//!
//! **The tree is what the host has said so far, never a guess.** A folder is listed one level at a
//! time, on the expand that asks for it, so a project with a `node_modules` in it costs one row
//! rather than a walk. A folder therefore has three states a flattened row has to carry apart —
//! shut, open, and waiting for its listing — and it has a twisty before anything is known about
//! it, because a listing says what is inside a folder and not whether there is anything.
//!
//! **Git state is an `Option`, and `None` is not "clean".** Nothing reads a repository yet, so a
//! row has no status to show rather than a status meaning nothing is wrong — the two look the same
//! and claim different things, and only one of them is true today.
//!
//! Everything here is a tree, a merge and a flatten: no frame, no bus, no path on disk. That is
//! what lets `tests/explorer.rs` assert the restore rules without a window.

use ubiq_proto::files::{DirEntry, DirListing, EntryKind};

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
}

#[derive(Clone, Debug)]
pub enum NodeKind {
    Dir {
        /// What the host said is inside. Empty until it has said anything.
        children: Vec<FileNode>,
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
                children: Vec::new(),
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

/// One visible line of the tree, already flattened for rendering.
#[derive(Clone, Debug)]
pub struct Row {
    pub name: String,
    pub path: String,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
    /// A listing is on its way. The row says so rather than looking like an empty folder.
    pub loading: bool,
    /// The host's ceiling cut this folder's listing short.
    pub truncated: bool,
    pub git: Option<GitStatus>,
    pub readable: bool,
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
    pub root: Vec<FileNode>,
    pub selected: Option<String>,
    /// Whether the host's ceiling cut the project's top level short.
    pub truncated: bool,
    /// Whether the root has ever been listed. Without it an empty tree and a tree whose first
    /// listing has not arrived look identical, and every remembered folder would be dropped as
    /// gone in the frame before the host answers.
    root_listed: bool,
}

impl ExplorerState {
    /// A tree that knows nothing yet, which is every tree until the host answers.
    pub fn empty() -> Self {
        Self {
            root: Vec::new(),
            selected: None,
            truncated: false,
            root_listed: false,
        }
    }

    /// Whether the project's top level has arrived.
    pub fn is_listed(&self) -> bool {
        self.root_listed
    }

    /// The tree flattened to what is on screen: shut folders contribute no children, and a
    /// non-empty filter matches on the whole path rather than the leaf name.
    ///
    /// The filter is the window's rather than the tree's, because one field drives whichever
    /// project is on screen and a copy per tree would disagree with it on every switch.
    ///
    /// **Filtering is finding, not pruning.** A non-empty filter emits the rows that match and
    /// descends through every folder whether it matched or not, so what comes back is the set of
    /// matches — and a folder that matched is drawn open, because its children are worth seeing.
    /// Only what the host has already named can match, which is the honest answer for a tree that
    /// is listed a level at a time rather than a promise about folders nobody has opened.
    pub fn rows(&self, filter: &str) -> Vec<Row> {
        let mut out = Vec::new();
        let needle = filter.trim().to_lowercase();
        flatten(&self.root, 0, &needle, &mut out);
        out
    }

    /// Flip a folder open or shut, answering whether its children have to be asked for.
    pub fn toggle(&mut self, path: &str) -> Toggle {
        let Some(node) = node_mut(&mut self.root, path) else {
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
        if let Some(node) = node_mut(&mut self.root, path)
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
        if listing.rel_path.is_empty() {
            merge_children(&mut self.root, listing.entries);
            self.root_listed = true;
            self.truncated = listing.truncated;
            return true;
        }

        let Some(node) = node_mut(&mut self.root, &listing.rel_path) else {
            return false;
        };
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

        merge_children(children, listing.entries);
        *listed = true;
        *loading = false;
        *truncated = listing.truncated;
        true
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
        let root = &mut self.root;
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
        collapse_in(&mut self.root);
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

fn flatten(nodes: &[FileNode], depth: usize, needle: &str, out: &mut Vec<Row>) {
    for node in nodes {
        let (expanded, loading, truncated) = match &node.kind {
            NodeKind::Dir {
                expanded,
                loading,
                truncated,
                ..
            } => (*expanded, *loading, *truncated),
            NodeKind::File => (false, false, false),
        };

        // A filtered tree shows matching files with their folders forced open; an unfiltered one
        // shows exactly what the user has expanded.
        let matches = needle.is_empty() || node.path.to_lowercase().contains(needle);
        if matches {
            out.push(Row {
                name: node.name.clone(),
                path: node.path.clone(),
                depth,
                is_dir: node.is_dir(),
                expanded: expanded || !needle.is_empty(),
                loading,
                truncated,
                git: node.git,
                readable: node.readable,
            });
        }

        if let NodeKind::Dir { children, .. } = &node.kind
            && (expanded || !needle.is_empty())
        {
            flatten(children, depth + 1, needle, out);
        }
    }
}

fn node_mut<'a>(nodes: &'a mut [FileNode], path: &str) -> Option<&'a mut FileNode> {
    for node in nodes.iter_mut() {
        if node.path == path {
            return Some(node);
        }
        if let NodeKind::Dir { children, .. } = &mut node.kind
            && let Some(found) = node_mut(children, path)
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
            collapse_in(children);
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
        nodes = children;
    }

    Reach::Gone
}
