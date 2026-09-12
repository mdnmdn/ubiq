use super::*;

impl ExplorerState {
    /// Flip a folder open or shut, answering whether its children have to be asked for.
    pub fn toggle(&mut self, path: &str) -> Toggle {
        // The empty path is the project's own row, which has no node behind it: what it opens and
        // shuts is the tree itself, and the host has already been asked for the top level.
        if path.is_empty() {
            self.root_expanded = !self.root_expanded;
            return Toggle::Done;
        }
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

    /// Open or shut a folder **as it is drawn**, which is not always the same as the folder's own
    /// flag: while a filter is typed every matching branch is drawn open, and shutting one records
    /// a per-filter override instead of changing what the window writes down. The answer says
    /// which happened, because a re-walk is what redraws an override and a listing is what
    /// redraws a real expand.
    pub fn toggle_drawn(&mut self, path: &str, filter: &str) -> Toggle {
        if filter.trim().is_empty() || self.view != ExplorerView::Tree {
            return self.toggle(path);
        }
        // The project's own row has no override to carry: what it shuts is the whole tree, and
        // that is the same flag filtered or not.
        if path.is_empty() {
            self.root_expanded = !self.root_expanded;
            return Toggle::Refiltered;
        }
        if node_of(&self.root, path).is_none() {
            return Toggle::Missing;
        }
        if !self.filter_collapsed.remove(path) {
            self.filter_collapsed.insert(path.to_string());
        }
        Toggle::Refiltered
    }

    /// Whether a folder is drawn open right now — the override under a filter, the folder's own
    /// flag otherwise. What the keyboard's left and right arrows read.
    pub(super) fn drawn_expanded(&self, path: &str, filter: &str) -> bool {
        if filter.trim().is_empty() || self.view != ExplorerView::Tree {
            return self.is_expanded(path);
        }
        if path.is_empty() {
            return self.root_expanded;
        }
        !self.filter_collapsed.contains(path)
    }

    /// Show or hide dotfiles. Answers whether it changed, and drops the hits when it did: they
    /// were walked under the other rule and the caller has to run the walk again.
    pub fn set_show_hidden(&mut self, on: bool) -> bool {
        if self.show_hidden == on {
            return false;
        }
        self.show_hidden = on;
        self.filter_hits = None;
        self.filter_job = self.filter_job.wrapping_add(1);
        true
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
    ///
    /// `repos` are the repositories inside the project. Their paths are already merged into
    /// `entries` and `rollups`; what the list adds is the boundary — a row to draw a branch on,
    /// and a place for inheritance to stop. Only a **managed** repository enters `git_repos`: an
    /// ignored one carries no marks in `entries`/`rollups` either, so drawing its boundary would
    /// only stop inheritance at a folder that has nothing to inherit from in the first place.
    pub fn apply_git(
        &mut self,
        generation: u64,
        entries: &[GitEntry],
        rollups: &[GitRollup],
        repos: &[GitNested],
    ) -> bool {
        if self.git_known && generation < self.git_generation {
            return false;
        }
        self.git_generation = generation;
        self.git_known = true;
        self.git_marks.clear();
        self.git_inherit.clear();
        self.git_repos = repos
            .iter()
            .filter(|repo| repo.managed)
            .map(|repo| {
                (
                    repo.rel_path.trim_end_matches('/').to_string(),
                    NestedRepo {
                        head: crate::state::git::head_label(&repo.head),
                        submodule: repo.submodule,
                    },
                )
            })
            .collect();
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
        self.git_repos.clear();
        self.paint_git();
    }

    fn paint_git(&mut self) {
        let known = self.git_known;
        paint_nodes(
            cow(&mut self.root),
            known,
            &self.git_marks,
            &self.git_inherit,
            &self.git_repos,
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
            self.cursor = Self::first_cursor(&rows);
        }
        self.sync_hit_cursors();
    }

    /// Folders the background cache still cannot see into, and which it is allowed to ask about.
    ///
    /// A name in [`WALK_SKIP`] is left alone: the host would not descend into it on a deep walk,
    /// and asking for it explicitly would list `node_modules` in full. Folders already asked about
    /// are skipped too, so a failed listing is not asked twice.
    /// A hidden folder is left alone too while the dotfile switch is off: listing a folder the
    /// tree will not draw is a walk nobody asked for.
    pub fn unlisted_for_cache(&self) -> Vec<String> {
        let mut out = Vec::new();
        collect_cache(&self.root, &self.cache_asked, self.show_hidden, &mut out);
        out
    }

    /// Folders a filter matched that the host has never listed — a hit the user sees as a folder
    /// with nothing under it, because nothing under it is known yet.
    ///
    /// The walk's skip set is left alone, exactly as the background cache leaves it alone: a
    /// search for `node_modules` is not a request to list it. A folder shut under the filter is
    /// left alone as well — it is drawn shut, so what is inside it is not on screen to ask for,
    /// and the rows under it are not in `rows` at all.
    pub fn unlisted_hits(&self, rows: &[Row]) -> Vec<String> {
        rows.iter()
            .filter(|row| row.is_dir && row.expanded && !row.path.is_empty() && row.readable)
            .filter(|row| !self.cache_asked.contains(&row.path) && !walk_skipped(&row.path))
            .filter(|row| !self.is_folder_listed(&row.path))
            .map(|row| row.path.clone())
            .collect()
    }

    /// Note that the cache has asked about these folders. They are marked loading so an expand
    /// while the answer is in flight does not ask again; a shut folder does not draw as spinning.
    pub fn begin_cache(&mut self, paths: &[String]) {
        for path in paths {
            self.cache_asked.insert(path.clone());
            self.set_loading(path, true);
        }
    }

    pub(super) fn is_expanded(&self, path: &str) -> bool {
        if path.is_empty() {
            return self.root_expanded;
        }
        match node_of(&self.root, path) {
            Some(node) => matches!(node.kind, NodeKind::Dir { expanded: true, .. }),
            None => false,
        }
    }

    pub(super) fn needs_listing(&self, path: &str) -> bool {
        // The project's top level is asked for when the project opens, never by a toggle.
        if path.is_empty() {
            return false;
        }
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

    /// The folder an action on this row lands in: the row itself when it is a folder, otherwise
    /// the folder holding it. Empty is the project's root.
    pub fn target_dir(&self, path: &str) -> String {
        // The project's own row, said rather than fallen out of the rsplit below.
        if path.is_empty() {
            return String::new();
        }
        let is_dir = node_of(&self.root, path).is_some_and(FileNode::is_dir);
        if is_dir {
            return path.to_string();
        }
        match path.rsplit_once('/') {
            Some((parent, _)) => parent.to_string(),
            None => String::new(),
        }
    }

    /// `leaf` inside `parent`, with a suffix if that name is taken among the children the host has
    /// already named. `notes.md` becomes `notes copy.md`, then `notes copy 2.md`.
    ///
    /// Best-effort by construction: it can only see a folder that has been listed, so a collision
    /// it misses comes back as `FileError::Conflict` and is shown on the row. The point is that
    /// Duplicate, where a collision is *certain*, does not need a round trip to discover it.
    pub fn free_name(&self, parent: &str, leaf: &str) -> String {
        let taken: Vec<&str> = match parent.is_empty() {
            true => self.root.iter().map(|node| node.name.as_str()).collect(),
            false => match node_of(&self.root, parent) {
                Some(FileNode {
                    kind: NodeKind::Dir { children, .. },
                    ..
                }) => children.iter().map(|node| node.name.as_str()).collect(),
                _ => Vec::new(),
            },
        };
        if !taken.contains(&leaf) {
            return leaf.to_string();
        }

        // The extension is kept on the end, because that is what says how to open the copy.
        let (stem, ext) = match leaf.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
            _ => (leaf, String::new()),
        };
        for n in 1.. {
            let candidate = match n {
                1 => format!("{stem} copy{ext}"),
                n => format!("{stem} copy {n}{ext}"),
            };
            if !taken.contains(&candidate.as_str()) {
                return candidate;
            }
        }
        unreachable!("the suffix grows without bound, so some name is free")
    }

    // ── the menu ────────────────────────────────────────────────────
}

pub(super) fn cow(nodes: &mut Arc<Vec<FileNode>>) -> &mut Vec<FileNode> {
    Arc::make_mut(nodes)
}

pub(super) fn paint_nodes(
    nodes: &mut [FileNode],
    known: bool,
    marks: &HashMap<String, GitStatus>,
    inherit_from: &HashSet<String>,
    repos: &HashMap<String, NestedRepo>,
    inherited: Option<GitStatus>,
) {
    for node in nodes {
        node.git = if known {
            marks.get(&node.path).copied().or(inherited)
        } else {
            None
        };
        node.repo = repos.get(&node.path).cloned();
        if let NodeKind::Dir { children, .. } = &mut node.kind {
            // A nested repository is where inheritance stops: what is inside it is accounted for
            // by its own repository and is already in the map, so an outer untracked or ignored
            // status must not be pushed across the boundary.
            let next = if node.repo.is_some() {
                None
            } else if inherit_from.contains(&node.path) {
                node.git
            } else {
                inherited
            };
            paint_nodes(cow(children), known, marks, inherit_from, repos, next);
        }
    }
}

pub(super) fn merge_children(existing: &mut Vec<FileNode>, entries: Vec<DirEntry>) {
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

pub(super) fn dir_flags(node: &FileNode) -> (bool, bool, bool) {
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

pub(super) fn walk_skipped(path: &str) -> bool {
    path.split('/').any(|part| WALK_SKIP.contains(&part))
}

/// A hidden entry, by the only rule there is: the wire carries no flag, so the interface reads the
/// name — the same test the host's own browse family applies.
pub(super) fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

pub(super) fn collect_cache(
    nodes: &[FileNode],
    asked: &HashSet<String>,
    show_hidden: bool,
    out: &mut Vec<String>,
) {
    for node in nodes {
        if !show_hidden && is_hidden(&node.name) {
            continue;
        }
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
            collect_cache(children, asked, show_hidden, out);
            continue;
        }
        if !node.readable || *loading || asked.contains(&node.path) || walk_skipped(&node.path) {
            continue;
        }
        out.push(node.path.clone());
    }
}

pub(super) fn collect_listed<'a>(
    nodes: &'a [FileNode],
    needle: &str,
    show_hidden: bool,
    out: &mut Vec<&'a FileNode>,
) {
    for node in nodes {
        if !show_hidden && is_hidden(&node.name) {
            continue;
        }
        if needle.is_empty() || node.path.to_lowercase().contains(needle) {
            out.push(node);
        }
        if let NodeKind::Dir { children, .. } = &node.kind {
            collect_listed(children, needle, show_hidden, out);
        }
    }
}

pub(super) fn node_of<'a>(nodes: &'a [FileNode], path: &str) -> Option<&'a FileNode> {
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

pub(super) fn node_mut<'a>(nodes: &'a mut [FileNode], path: &str) -> Option<&'a mut FileNode> {
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

pub(super) fn collect_expanded(nodes: &[FileNode], out: &mut Vec<String>) {
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

pub(super) fn collapse_in(nodes: &mut [FileNode]) {
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
pub(super) fn reach(nodes: &mut Vec<FileNode>, root_listed: bool, path: &str) -> Reach {
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

/// The folder an entry is in, as the list writes it. The project root is `.`, because a blank
/// column would read as missing rather than as "at the top".
pub(super) fn parent_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((parent, _)) if !parent.is_empty() => parent.to_string(),
        _ => ".".to_string(),
    }
}
