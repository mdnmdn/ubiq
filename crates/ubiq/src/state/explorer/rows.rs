use super::tree::*;
use super::*;

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
    pub(super) fn visible_rows(&self, filter: &str) -> Vec<Row> {
        if filter.trim().is_empty() {
            return self.rows("");
        }
        self.hits_for(filter).unwrap_or_else(|| self.rows(filter))
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
}
