//! The file tree the explorer draws, and the git state each row carries.

/// How a file stands against the index. The explorer tints the name and shows a single-letter
/// badge from this, never from wording alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GitStatus {
    Clean,
    Modified,
    Untracked,
    Conflict,
    Staged,
    Ignored,
}

impl GitStatus {
    /// The badge shown at the end of the row, if any.
    pub fn badge(self) -> Option<&'static str> {
        match self {
            GitStatus::Clean => None,
            GitStatus::Modified => Some("M"),
            GitStatus::Untracked => Some("U"),
            GitStatus::Conflict => Some("!"),
            GitStatus::Staged => Some("S"),
            GitStatus::Ignored => Some("ignored"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum NodeKind {
    Dir {
        children: Vec<FileNode>,
        expanded: bool,
    },
    File,
}

#[derive(Clone, Debug)]
pub struct FileNode {
    pub name: String,
    pub path: String,
    pub kind: NodeKind,
    pub git: GitStatus,
}

impl FileNode {
    pub fn file(path: &str, git: GitStatus) -> Self {
        Self {
            name: leaf(path).to_string(),
            path: path.to_string(),
            kind: NodeKind::File,
            git,
        }
    }

    pub fn dir(path: &str, git: GitStatus, expanded: bool, children: Vec<FileNode>) -> Self {
        Self {
            name: leaf(path).to_string(),
            path: path.to_string(),
            kind: NodeKind::Dir { children, expanded },
            git,
        }
    }

    fn is_dir(&self) -> bool {
        matches!(self.kind, NodeKind::Dir { .. })
    }
}

fn leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// One visible line of the tree, already flattened for rendering.
#[derive(Clone, Debug)]
pub struct Row {
    pub name: String,
    pub path: String,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
    pub git: GitStatus,
}

pub struct ExplorerState {
    pub root: Vec<FileNode>,
    pub selected: Option<String>,
    pub filter: String,
}

impl ExplorerState {
    pub fn new(root: Vec<FileNode>) -> Self {
        Self {
            root,
            selected: None,
            filter: String::new(),
        }
    }

    /// The tree flattened to what is on screen: collapsed folders contribute no children, and a
    /// non-empty filter matches on the path rather than the leaf name.
    pub fn rows(&self) -> Vec<Row> {
        let mut out = Vec::new();
        let needle = self.filter.trim().to_lowercase();
        flatten(&self.root, 0, &needle, &mut out);
        out
    }

    pub fn toggle(&mut self, path: &str) {
        toggle_in(&mut self.root, path);
    }

    pub fn collapse_all(&mut self) {
        collapse_in(&mut self.root);
    }
}

fn flatten(nodes: &[FileNode], depth: usize, needle: &str, out: &mut Vec<Row>) {
    for node in nodes {
        let expanded = match &node.kind {
            NodeKind::Dir { expanded, .. } => *expanded,
            NodeKind::File => false,
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
                git: node.git,
            });
        }

        if let NodeKind::Dir { children, .. } = &node.kind
            && (expanded || !needle.is_empty())
        {
            flatten(children, depth + 1, needle, out);
        }
    }
}

fn toggle_in(nodes: &mut [FileNode], path: &str) -> bool {
    for node in nodes {
        if node.path == path {
            if let NodeKind::Dir { expanded, .. } = &mut node.kind {
                *expanded = !*expanded;
            }
            return true;
        }
        if let NodeKind::Dir { children, .. } = &mut node.kind
            && toggle_in(children, path)
        {
            return true;
        }
    }
    false
}

fn collapse_in(nodes: &mut [FileNode]) {
    for node in nodes {
        if let NodeKind::Dir { children, expanded } = &mut node.kind {
            *expanded = false;
            collapse_in(children);
        }
    }
}
