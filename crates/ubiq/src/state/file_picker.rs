//! The file picker: what it was asked for, what it is showing, and what has been picked.
//!
//! **One picker, asked for in six ways.** A screen that needs a path says what it wants — files or
//! folders, one or many, from which folder down, through which prefilter, final on the click or on
//! the button, holding the window or dismissed by a click outside — and gets the same dialog every
//! time. The request is [`PickerRequest`]; everything else here is what the dialog does with it.
//!
//! **Tree and list are the same set, arranged twice.** The tree is the folders the user walked
//! into; the list is every match under the root, flat, each with the folder it came from. Which one
//! is on screen is the user's choice and nothing else's — a picker that decided for them would be a
//! picker that hides the file they can see the name of.
//!
//! **Nothing here reads a disk.** The forest is handed in by whoever raised the picker, which is
//! what lets the kitchen sink raise one with no project open and what will one day let the host's
//! listings fill the same dialog. Paths are project-relative, as every path the interface holds is.

use std::collections::HashSet;

/// What the picker hands back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickKind {
    Files,
    Folders,
}

impl PickKind {
    /// What the header's chip says the dialog is for.
    pub fn label(self) -> &'static str {
        match self {
            PickKind::Files => "files",
            PickKind::Folders => "folders",
        }
    }

    /// Whether a row of this kind is an answer rather than a way to one. A folder is drawn in both
    /// modes — it is how the files are reached — and is only ever picked in one of them.
    pub fn picks(self, is_dir: bool) -> bool {
        match self {
            PickKind::Files => !is_dir,
            PickKind::Folders => is_dir,
        }
    }
}

/// How the same set is arranged. The user's choice, kept while the dialog is up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickerView {
    Tree,
    List,
}

impl PickerView {
    pub fn label(self) -> &'static str {
        match self {
            PickerView::Tree => "Tree view",
            PickerView::List => "List view",
        }
    }
}

/// One answer, or several.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickerCount {
    Single,
    Multiple,
}

/// When a single pick is final. Meaningless for a multiple pick, which always has a button under
/// it: there is no click that could mean "and that is all of them".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Commit {
    OnClick,
    OnButton,
}

/// Who raised the picker, and therefore who is owed what comes back.
///
/// The dialog itself belongs to the window, so the answer has to be routed rather than returned.
/// One variant today, because one screen asks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickerOwner {
    Sink,
}

/// Everything a caller says when it raises a picker.
#[derive(Clone, Debug)]
pub struct PickerRequest {
    pub owner: PickerOwner,
    /// What the dialog's header calls it. A sentence about what is being chosen, not "File picker".
    pub title: String,
    /// The folder the dialog opens on, project-relative. Empty is the project itself.
    pub root: String,
    /// The prefilter a caller applies before the user types anything: `*.md`, `*.rs`, `Cargo.*`.
    /// It matches file names only — a folder is never hidden by it, or the files under it could
    /// not be reached.
    pub pattern: Option<String>,
    pub kind: PickKind,
    pub count: PickerCount,
    pub commit: Commit,
    /// Whether the dialog holds the window. A picker that is not modal is dismissed by a click
    /// anywhere outside it.
    pub modal: bool,
}

impl PickerRequest {
    /// The usual ask: several files, from the project root, final on the button, holding the
    /// window.
    pub fn new(owner: PickerOwner, title: impl Into<String>) -> Self {
        Self {
            owner,
            title: title.into(),
            root: String::new(),
            pattern: None,
            kind: PickKind::Files,
            count: PickerCount::Multiple,
            commit: Commit::OnButton,
            modal: true,
        }
    }

    pub fn root(mut self, root: impl Into<String>) -> Self {
        self.root = root.into();
        self
    }

    pub fn pattern(mut self, pattern: Option<&str>) -> Self {
        self.pattern = pattern.map(str::to_string);
        self
    }

    pub fn kind(mut self, kind: PickKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn count(mut self, count: PickerCount) -> Self {
        self.count = count;
        self
    }

    pub fn commit(mut self, commit: Commit) -> Self {
        self.commit = commit;
        self
    }

    pub fn modal(mut self, modal: bool) -> Self {
        self.modal = modal;
        self
    }

    /// Whether a click on a pickable row is the whole answer. A multiple pick never is, however it
    /// was asked for.
    pub fn commits_on_click(&self) -> bool {
        self.count == PickerCount::Single && self.commit == Commit::OnClick
    }
}

/// One entry in the forest the picker was handed. A file has no children; a folder always has the
/// vector, empty or not.
#[derive(Clone, Debug)]
pub struct PickerNode {
    pub name: String,
    /// Project-relative. The project's own node carries the empty path.
    pub path: String,
    /// What the tree reports at the end of the row. Folders report nothing.
    pub size: Option<u64>,
    children: Option<Vec<PickerNode>>,
}

impl PickerNode {
    pub fn file(name: &str, path: &str, size: u64) -> Self {
        Self {
            name: name.to_string(),
            path: path.to_string(),
            size: Some(size),
            children: None,
        }
    }

    pub fn dir(name: &str, path: &str, children: Vec<PickerNode>) -> Self {
        Self {
            name: name.to_string(),
            path: path.to_string(),
            size: None,
            children: Some(children),
        }
    }

    pub fn is_dir(&self) -> bool {
        self.children.is_some()
    }

    pub fn children(&self) -> &[PickerNode] {
        self.children.as_deref().unwrap_or(&[])
    }
}

/// One visible line, already arranged for whichever view is on screen.
#[derive(Clone, Debug)]
pub struct PickerRow {
    pub name: String,
    pub path: String,
    /// How far in the tree indents it. Always zero in the list, which is what flat means.
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
    pub selected: bool,
    /// Whether the keyboard is on this row. Selection is what comes back; the cursor is only where
    /// the next key lands, and the two are drawn differently because they mean different things.
    pub on_cursor: bool,
    /// Whether choosing this row is what the picker was asked for.
    pub pickable: bool,
    /// What the row says at its far end: how big it is in the tree, which folder it is in in the
    /// list.
    pub trailing: String,
}

/// A key the dialog answers to, told apart from the ones it does not.
///
/// The keystrokes themselves are `ui::file_picker`'s: what a platform calls "confirm" is not
/// something a picker's rules should have an opinion about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickerKey {
    Up,
    Down,
    /// Shut the folder the keyboard is on, or step out to the one holding it.
    Left,
    /// Open the folder the keyboard is on, or step into it.
    Right,
    /// Tick the row the keyboard is on — or, where one answer was asked for, be the answer.
    Enter,
    /// Hand back what has been picked, however many that is.
    Confirm,
    Dismiss,
}

/// What a key press turned out to mean for the window holding the dialog.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pressed {
    /// Nothing here answers this key. Whoever else wants it may have it — which is how `left` and
    /// `right` go back to being the filter field's caret keys in the flat list.
    Ignored,
    /// The cursor moved, a folder opened, or a row was ticked.
    Moved,
    Commit,
    Dismiss,
}

/// What the dialog may be shrunk to, what it opens at, and what it may be grown to. The maximum is
/// a fraction of the window rather than a size, because the window is what it has to fit inside.
pub const MIN_WIDTH: f32 = 420.0;
pub const MIN_HEIGHT: f32 = 300.0;
pub const DEFAULT_WIDTH: f32 = 660.0;
pub const DEFAULT_HEIGHT: f32 = 560.0;

/// The picker that is up.
pub struct FilePickerState {
    pub request: PickerRequest,
    pub view: PickerView,
    /// What the field above the rows holds. The field itself is the window's, as every field is.
    pub filter: String,
    /// The forest the dialog draws: the requested folder, or everything when none was asked for.
    forest: Vec<PickerNode>,
    expanded: HashSet<String>,
    /// In pick order, because "Add 2" adds them in the order they were ticked.
    picked: Vec<String>,
    /// Which row the keyboard is on. A path rather than an index: rows come and go as folders open
    /// and the filter narrows, and an index would be pointing at a different row afterwards.
    cursor: Option<String>,
    pub width: f32,
    pub height: f32,
    /// Where the corner grip went down, and how big the dialog was then. A resize is measured from
    /// where it started rather than from the last frame, so a drag that outruns the pointer does
    /// not drift.
    drag: Option<((f32, f32), (f32, f32))>,
}

impl FilePickerState {
    /// Raise a picker over `forest`, showing the folder the request named.
    ///
    /// A request naming a folder nobody handed in opens on the whole forest rather than on
    /// nothing: an empty dialog says the project is empty, which would be a lie.
    pub fn open(request: PickerRequest, forest: Vec<PickerNode>, view: PickerView) -> Self {
        let rooted = match request.root.is_empty() {
            true => forest,
            false => match find(&forest, &request.root) {
                Some(node) => vec![node.clone()],
                None => forest,
            },
        };

        // The top of what is shown opens with it. A dialog whose only row is a shut folder makes
        // the user click once to see anything at all.
        let expanded = rooted
            .iter()
            .filter(|node| node.is_dir())
            .map(|node| node.path.clone())
            .collect();

        let mut state = Self {
            request,
            view,
            filter: String::new(),
            forest: rooted,
            expanded,
            picked: Vec::new(),
            cursor: None,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            drag: None,
        };

        // The keyboard starts on the first row, so the first arrow moves rather than appears.
        state.cursor = state.rows().first().map(|row| row.path.clone());
        state
    }

    // ── what is on screen ───────────────────────────────────────────

    /// The rows the dialog draws, in the view it is in.
    pub fn rows(&self) -> Vec<PickerRow> {
        let needle = self.filter.trim().to_lowercase();
        match self.view {
            PickerView::Tree => {
                let mut out = Vec::new();
                self.tree_rows(&self.forest, 0, &needle, &mut out);
                out
            }
            PickerView::List => self.list_rows(&needle),
        }
    }

    /// The tree: folders the user walked into, and the entries the prefilter left in them.
    ///
    /// **A filter finds rather than prunes.** While one is typed every folder is walked whether it
    /// was opened or not, and a folder with nothing matching under it is left out — otherwise the
    /// answer to a search is a screen of empty folders.
    fn tree_rows(
        &self,
        nodes: &[PickerNode],
        depth: usize,
        needle: &str,
        out: &mut Vec<PickerRow>,
    ) {
        for node in nodes {
            if node.is_dir() {
                if !needle.is_empty() && !self.subtree_matches(node, needle) {
                    continue;
                }
                let expanded = self.expanded.contains(&node.path) || !needle.is_empty();
                out.push(self.row(node, depth, expanded, String::new()));
                if expanded {
                    self.tree_rows(node.children(), depth + 1, needle, out);
                }
                continue;
            }

            // Folders-only never draws a file: a row that cannot be the answer and leads nowhere is
            // noise in a dialog whose whole job is to be scanned.
            if self.request.kind == PickKind::Folders || !self.shows(node, needle) {
                continue;
            }
            out.push(self.row(node, depth, false, size_label(node.size)));
        }
    }

    /// The list: every match under the root, flat, each said to be in the folder it is in.
    ///
    /// Sorted by name without case, because a flat list is read by name — the folder is the answer
    /// to "which one is this", not the thing the eye is scanning.
    fn list_rows(&self, needle: &str) -> Vec<PickerRow> {
        let mut flat = Vec::new();
        self.flatten(&self.forest, needle, &mut flat);
        flat.sort_by_key(|node| node.name.to_lowercase());
        flat.into_iter()
            .map(|node| {
                let folder = parent_of(&node.path);
                self.row(node, 0, false, folder)
            })
            .collect()
    }

    fn flatten<'a>(&self, nodes: &'a [PickerNode], needle: &str, out: &mut Vec<&'a PickerNode>) {
        for node in nodes {
            if node.is_dir() {
                // The folder the dialog is rooted at is the ground everything is measured from,
                // never a row of its own in the flat view.
                if self.request.kind == PickKind::Folders
                    && !node.path.is_empty()
                    && self.shows(node, needle)
                {
                    out.push(node);
                }
                self.flatten(node.children(), needle, out);
            } else if self.request.kind == PickKind::Files && self.shows(node, needle) {
                out.push(node);
            }
        }
    }

    /// Whether an entry survives both filters: the caller's prefilter, and what the user typed.
    fn shows(&self, node: &PickerNode, needle: &str) -> bool {
        if !node.is_dir()
            && let Some(pattern) = &self.request.pattern
            && !matches_glob(pattern, &node.name)
        {
            return false;
        }
        needle.is_empty() || node.path.to_lowercase().contains(needle)
    }

    /// Whether anything the dialog would draw sits under this folder — the folder itself included,
    /// so searching for a folder's name finds it.
    fn subtree_matches(&self, node: &PickerNode, needle: &str) -> bool {
        if self.shows(node, needle) {
            return true;
        }
        node.children().iter().any(|child| match child.is_dir() {
            true => self.subtree_matches(child, needle),
            false => self.request.kind == PickKind::Files && self.shows(child, needle),
        })
    }

    fn row(&self, node: &PickerNode, depth: usize, expanded: bool, trailing: String) -> PickerRow {
        PickerRow {
            name: node.name.clone(),
            path: node.path.clone(),
            depth,
            is_dir: node.is_dir(),
            expanded,
            selected: self.picked.contains(&node.path),
            on_cursor: self.cursor.as_deref() == Some(node.path.as_str()),
            pickable: self.request.kind.picks(node.is_dir()),
            trailing,
        }
    }

    // ── what the user does to it ────────────────────────────────────

    pub fn set_view(&mut self, view: PickerView) {
        self.view = view;
        self.reanchor();
    }

    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.reanchor();
    }

    /// Put the keyboard back on a row that is still there.
    ///
    /// A filter narrows the rows and the two views are not the same set — the tree's root folder is
    /// no row at all in the flat list — so a cursor left where it was would be pointing at nothing,
    /// and the next arrow would land somewhere the user did not come from.
    fn reanchor(&mut self) {
        let rows = self.rows();
        let held = self
            .cursor
            .as_deref()
            .is_some_and(|path| rows.iter().any(|row| row.path == path));
        if !held {
            self.cursor = rows.first().map(|row| row.path.clone());
        }
    }

    pub fn toggle_folder(&mut self, path: &str) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_string());
        }
    }

    /// Tick or untick a row, answering whether that was the whole answer.
    ///
    /// A single pick replaces whatever was there — which is what makes it single — and is final on
    /// the click only when the caller asked for that.
    pub fn pick(&mut self, path: &str) -> bool {
        match self.request.count {
            PickerCount::Single => {
                self.picked = vec![path.to_string()];
                self.request.commits_on_click()
            }
            PickerCount::Multiple => {
                match self.picked.iter().position(|held| held == path) {
                    Some(at) => {
                        self.picked.remove(at);
                    }
                    None => self.picked.push(path.to_string()),
                }
                false
            }
        }
    }

    /// What a click on a row means: a folder that cannot be picked opens instead of being chosen,
    /// which is the only way to reach what is inside it in the tree.
    pub fn click(&mut self, path: &str) -> bool {
        // The keyboard follows the mouse: an arrow after a click carries on from the row that was
        // clicked, not from wherever the cursor was left.
        self.cursor = Some(path.to_string());

        let pickable = self
            .node(path)
            .map(|node| self.request.kind.picks(node.is_dir()))
            .unwrap_or(false);

        if pickable {
            return self.pick(path);
        }
        if self.view == PickerView::Tree {
            self.toggle_folder(path);
        }
        false
    }

    fn node(&self, path: &str) -> Option<&PickerNode> {
        find(&self.forest, path)
    }

    // ── the keyboard ────────────────────────────────────────────────

    /// Which row the keyboard is on, and where it is in what is drawn.
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// Where the cursor sits in the rows on screen, which is what a scroll has to be told.
    pub fn cursor_index(&self) -> Option<usize> {
        self.index_in(&self.rows())
    }

    /// What a key means here, and what is left for whoever else wants it.
    ///
    /// **Every rule is in this one function**, so the dialog behaves the same however the key
    /// arrived — and so `tests/file_picker.rs` can press keys without a window.
    pub fn press(&mut self, key: PickerKey) -> Pressed {
        match key {
            PickerKey::Dismiss => Pressed::Dismiss,
            PickerKey::Up => self.step(-1),
            PickerKey::Down => self.step(1),
            PickerKey::Left => self.step_out(),
            PickerKey::Right => self.step_in(),
            PickerKey::Enter => self.enter(),
            // Nothing chosen is nothing to hand back, and a dialog that closed on it would be
            // answering a question nobody asked.
            PickerKey::Confirm => match self.can_commit() {
                true => Pressed::Commit,
                false => Pressed::Ignored,
            },
        }
    }

    fn index_in(&self, rows: &[PickerRow]) -> Option<usize> {
        let cursor = self.cursor.as_deref()?;
        rows.iter().position(|row| row.path == cursor)
    }

    /// One row up or down, stopping at the ends rather than wrapping — a list that wraps loses the
    /// user the moment they hold the key down.
    fn step(&mut self, delta: isize) -> Pressed {
        let rows = self.rows();
        if rows.is_empty() {
            return Pressed::Ignored;
        }
        let next = match self.index_in(&rows) {
            Some(at) => (at as isize + delta).clamp(0, rows.len() as isize - 1) as usize,
            // Nothing to carry on from: down lands on the first row and up on the last.
            None if delta > 0 => 0,
            None => rows.len() - 1,
        };
        self.cursor = Some(rows[next].path.clone());
        Pressed::Moved
    }

    /// Open the folder the cursor is on, or — where it is already open — step into it.
    fn step_in(&mut self) -> Pressed {
        let rows = self.rows();
        let Some(at) = self.index_in(&rows) else {
            return Pressed::Ignored;
        };
        let row = &rows[at];
        // The flat list has no depth to walk into, so the key goes back to the field.
        if self.view != PickerView::Tree || !row.is_dir {
            return Pressed::Ignored;
        }

        if !row.expanded {
            self.expanded.insert(row.path.clone());
            return Pressed::Moved;
        }

        match rows.get(at + 1).filter(|next| next.depth > row.depth) {
            Some(child) => {
                self.cursor = Some(child.path.clone());
                Pressed::Moved
            }
            // An open folder with nothing under it: there is nowhere to step.
            None => Pressed::Ignored,
        }
    }

    /// Shut the folder the cursor is on, or step out to the folder holding it.
    ///
    /// The second half is what makes the key usable deep in a tree: from a file, `left` goes to its
    /// folder, and pressing it again shuts that folder.
    fn step_out(&mut self) -> Pressed {
        let rows = self.rows();
        let Some(at) = self.index_in(&rows) else {
            return Pressed::Ignored;
        };
        let row = &rows[at];
        if self.view != PickerView::Tree {
            return Pressed::Ignored;
        }

        // While a filter is typed every folder is drawn open, so shutting one would change nothing
        // on screen. Stepping out still means something, and that is what it does.
        if row.is_dir && self.expanded.contains(&row.path) && self.filter.trim().is_empty() {
            let path = row.path.clone();
            self.expanded.remove(&path);
            return Pressed::Moved;
        }

        let depth = row.depth;
        match rows[..at].iter().rposition(|above| above.depth < depth) {
            Some(parent) => {
                self.cursor = Some(rows[parent].path.clone());
                Pressed::Moved
            }
            None => Pressed::Ignored,
        }
    }

    /// What `enter` means on the row the keyboard is on.
    ///
    /// **A single pick is confirmed by it**, whether or not the click was: a dialog raised for one
    /// answer has nothing else to wait for once the answer is on screen. A multiple pick ticks the
    /// row and stays up, because "and that is all of them" is a second key — `secondary-enter`.
    fn enter(&mut self) -> Pressed {
        let rows = self.rows();
        let Some(at) = self.index_in(&rows) else {
            return Pressed::Ignored;
        };
        let row = rows[at].clone();

        if !row.pickable {
            // A folder in a files-only dialog: the only thing enter can mean on it is "open it".
            if row.is_dir && self.view == PickerView::Tree {
                self.toggle_folder(&row.path);
                return Pressed::Moved;
            }
            return Pressed::Ignored;
        }

        self.pick(&row.path);
        match self.request.count {
            PickerCount::Single => Pressed::Commit,
            PickerCount::Multiple => Pressed::Moved,
        }
    }

    // ── what comes back ─────────────────────────────────────────────

    pub fn picked(&self) -> &[String] {
        &self.picked
    }

    pub fn count(&self) -> usize {
        self.picked.len()
    }

    /// Whether there is anything to hand back. Nothing picked is a dialog with nothing to do.
    pub fn can_commit(&self) -> bool {
        !self.picked.is_empty()
    }

    /// What the footer says on the left: how much has been chosen, in the words for this ask.
    pub fn tally(&self) -> String {
        match (self.request.count, self.count()) {
            (PickerCount::Single, 0) => format!("No {} chosen", self.request.kind.label()),
            (PickerCount::Single, _) => self.picked[0].clone(),
            (_, count) => format!("{count} selected"),
        }
    }

    /// What the confirming button says. A button says what it does, and how many it will do it to.
    pub fn confirm_label(&self) -> String {
        match self.request.count {
            PickerCount::Single => "Select".to_string(),
            PickerCount::Multiple => match self.count() {
                0 => "Add".to_string(),
                count => format!("Add {count}"),
            },
        }
    }

    // ── how big it is ───────────────────────────────────────────────

    /// Put the dialog at a size, inside what it may be shrunk to and what the window can hold.
    pub fn resize(&mut self, width: f32, height: f32, viewport: (f32, f32)) {
        let max_w = (viewport.0 - 48.0).max(MIN_WIDTH);
        let max_h = (viewport.1 - 48.0).max(MIN_HEIGHT);
        self.width = width.clamp(MIN_WIDTH, max_w);
        self.height = height.clamp(MIN_HEIGHT, max_h);
    }

    /// Note where a corner drag began. Everything after it is measured from here.
    pub fn start_drag(&mut self, at: (f32, f32)) {
        self.drag = Some((at, (self.width, self.height)));
    }

    /// Follow the pointer, answering whether anything moved.
    pub fn drag_to(&mut self, at: (f32, f32), viewport: (f32, f32)) -> bool {
        let Some((from, size)) = self.drag else {
            return false;
        };
        // The grip is at the corner and the dialog is centred, so it grows twice as fast as the
        // pointer moves: the far edge moves the same distance the near one does.
        let (before_w, before_h) = (self.width, self.height);
        self.resize(
            size.0 + (at.0 - from.0) * 2.0,
            size.1 + (at.1 - from.1) * 2.0,
            viewport,
        );
        before_w != self.width || before_h != self.height
    }

    pub fn end_drag(&mut self) {
        self.drag = None;
    }

    pub fn is_resizing(&self) -> bool {
        self.drag.is_some()
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

/// How big a file is, in the unit a person reads it in.
pub fn size_label(size: Option<u64>) -> String {
    let Some(bytes) = size else {
        return String::new();
    };
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    match bytes {
        0..KB => format!("{bytes} B"),
        KB..MB => format!("{} KB", bytes / KB),
        _ => format!("{} MB", bytes / MB),
    }
}

fn find<'a>(nodes: &'a [PickerNode], path: &str) -> Option<&'a PickerNode> {
    for node in nodes {
        if node.path == path {
            return Some(node);
        }
        if let Some(found) = find(node.children(), path) {
            return Some(found);
        }
    }
    None
}

/// The prefilter's match: `*` for any run, `?` for one character, everything else itself, without
/// case.
///
/// Written here rather than taken from a crate because a picker's prefilter is a handful of
/// wildcards and never a path expression — `*.md`, `Cargo.*`, `?ain.rs` — and a dependency that
/// also understood `**/` and `{a,b}` would be a promise this dialog does not keep.
pub fn matches_glob(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let name: Vec<char> = name.to_lowercase().chars().collect();

    // The usual two-cursor walk: `star` remembers the last `*` to fall back to, so a mismatch
    // after one resumes with the star having eaten one more character.
    let (mut p, mut n) = (0usize, 0usize);
    let (mut star, mut resume) = (None, 0usize);

    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some(p);
                resume = n;
                p += 1;
            }
            Some('?') => {
                p += 1;
                n += 1;
            }
            Some(ch) if *ch == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some(at) => {
                    p = at + 1;
                    resume += 1;
                    n = resume;
                }
                None => return false,
            },
        }
    }

    pattern[p..].iter().all(|ch| *ch == '*')
}
