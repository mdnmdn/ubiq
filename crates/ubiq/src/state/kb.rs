//! The knowledge base a project keeps: its sources, the tree under each, and the documents open
//! from them.
//!
//! **An open document is an [`OpenFile`]**, the very type the IDE's tabs are, with
//! `OpenFile::kb_source` set — so it gets the editor, the highlighting, the viewer, the dirty
//! tracking and the layout toggle by being one rather than by restating any of them, and the
//! dock gives it the tab chrome. [`kb_tab_key`] is the key space that keeps a document and a
//! project file of the same path two tabs rather than one.
//!
//! This is the documents half of the IDE, and it differs from [`super::explorer`] in one
//! structural way: **there are several sources, and none of them need be the project**. A source
//! is a folder the user pointed at, a repository the host clones for them, or a wiki Ubiq keeps
//! itself — so a path here is meaningless without the [`KbSourceId`] beside it — every row, every
//! selection and every ask carries the pair.
//!
//! The tree per source is deliberately a smaller thing than `ExplorerState`: no git marks, no
//! filter walk and no drag. What it keeps is what a documents explorer needs — what is inside a
//! folder, whether it is open, whether its listing is still in flight — plus the one thing a
//! documents explorer cannot do without: a right-click menu, which is [`KbMenu`] and is the
//! explorer's own menu machinery said again for rows that carry a source beside their path.

use std::sync::Arc;

use ubiq_proto::files::{DirListing, EntryKind};
use ubiq_proto::ids::{KbSourceId, RepoQueryId};
use ubiq_proto::kb::{KbAccess, KbOrigin, KbSource, KbSourceState, KbSourceStatus, KbStore};
use ubiq_proto::repos::CloneError;

use super::editor::OpenFile;
use super::explorer::{FileNode, NodeKind};

/// One source as the screen holds it: what the host says it is, plus the tree under it.
pub struct KbSourceView {
    pub status: KbSourceStatus,
    /// The source row's own children. There is no node for the source itself, on
    /// `ExplorerState`'s reasoning: the row is synthesised from the status beside it.
    pub children: Arc<Vec<FileNode>>,
    pub expanded: bool,
    /// Whether the host has ever listed the source. Not the same as having children — a source
    /// that was listed and is empty is a different thing from one nobody asked about.
    pub listed: bool,
    pub loading: bool,
    pub truncated: bool,
    /// Why the last listing failed, when one did. Drawn on the source's row rather than raised.
    pub error: Option<String>,
}

impl KbSourceView {
    fn new(status: KbSourceStatus) -> Self {
        Self {
            status,
            children: Arc::new(Vec::new()),
            expanded: true,
            listed: false,
            loading: false,
            truncated: false,
            error: None,
        }
    }

    pub fn id(&self) -> KbSourceId {
        self.status.source.id
    }

    pub fn name(&self) -> &str {
        &self.status.source.name
    }

    /// The one line under the source's name: where it comes from.
    pub fn origin(&self) -> String {
        match &self.status.source.origin {
            KbOrigin::Folder { path } => path.clone(),
            KbOrigin::Git { url, branch, .. } => match branch {
                Some(branch) => format!("{url} · {branch}"),
                None => url.clone(),
            },
            KbOrigin::Internal => "Wiki".to_string(),
        }
    }
}

/// The knowledge base of the project on screen.
///
/// `configured` is not `!sources.is_empty()`: a project whose configuration has not arrived yet
/// has no sources either, and the two must not draw the same. The "Add KB" page is for the first,
/// a blank panel for the second.
#[derive(Default)]
pub struct KbState {
    pub sources: Vec<KbSourceView>,
    /// Whether the host has answered the configuration at all.
    pub loaded: bool,
    /// The document the explorer's highlight is on, as the pair that identifies one.
    pub selected: Option<KbDocKey>,
    /// The documents open as tabs, in the order they were opened.
    ///
    /// **The same [`OpenFile`] the IDE's tabs are**, so a document gets the IDE's viewer, its
    /// buffer, its highlighting, its dirty tracking and its layout toggle by being one rather than
    /// by restating any of it. What tells a document from a project file is `OpenFile::kb_source`,
    /// which is also what gives it a key of its own; the dock draws each as a
    /// `PanelKind::Kb` panel.
    pub docs: Vec<OpenFile>,
    /// The right-click menu, while one is up.
    pub menu: Option<KbMenu>,
    /// Stamped onto each menu as it opens, so the outside click that dismisses the old one cannot
    /// shut the new one raised by the same event — `ExplorerState::close_menu`'s reasoning.
    menu_epoch: u64,
}

/// What identifies one document: its source and its path inside that source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KbDocKey {
    pub source: KbSourceId,
    pub path: String,
}

impl KbDocKey {
    pub fn new(source: KbSourceId, path: impl Into<String>) -> Self {
        Self {
            source,
            path: path.into(),
        }
    }

    /// The file's name, which is what a tab and a header say.
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// The tab key this document is known by — its panel's, its entry in the Markdown renderer's
    /// scan cache, and what the dock's saved layout names it by.
    pub fn tab_key(&self) -> String {
        kb_tab_key(self.source, &self.path)
    }

    /// The same key read back, or nothing for a key that names no document.
    pub fn from_tab_key(key: &str) -> Option<Self> {
        let (source, path) = key.strip_prefix(KB_TAB_PREFIX)?.split_once(':')?;
        Some(Self::new(source.parse::<KbSourceId>().ok()?, path))
    }

    /// The document an open tab is, for a tab that is one.
    pub fn of(file: &OpenFile) -> Option<Self> {
        Some(Self::new(file.kb_source?, file.path.clone()))
    }
}

/// What every knowledge-base tab key starts with.
const KB_TAB_PREFIX: &str = "kb:";

/// The tab key for one document: **the knowledge base's own key space**, so a document and an
/// editor tab on a project file of the same name are two tabs, two panels and two entries in the
/// renderer's scan cache rather than one.
///
/// This is the one convention — [`OpenFile::key`], the dock's payload and every cache that keys
/// on a tab read it here rather than each improvising a format of its own.
pub fn kb_tab_key(source: KbSourceId, path: &str) -> String {
    format!("{KB_TAB_PREFIX}{source}:{path}")
}

/// The directory a path lives in, `ProjectPathEdited`'s own reasoning said for the knowledge
/// base: empty for a top-level path, the one place `KbChanged`'s `rel_path` is matched against a
/// document that might be mid-save.
pub fn kb_parent_path(path: &str) -> &str {
    path.rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("")
}

/// One row of the flattened tree the panel draws.
pub struct KbRow {
    pub source: KbSourceId,
    /// Empty for a source's own row.
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub kind: KbRowKind,
    pub selected: bool,
}

pub enum KbRowKind {
    /// A source's own row: its state decides what it says beside the name.
    Root {
        expanded: bool,
        state: KbSourceState,
        origin: String,
        /// The last refusal this source saw, from a listing or from a write. Drawn on the row
        /// because a write that failed has no document on screen to fail in, and a refusal only
        /// the log records is a gesture that silently did nothing.
        error: Option<String>,
    },
    Dir {
        expanded: bool,
        loading: bool,
    },
    File,
}

/// What a click on a row asks for.
pub enum KbPressed {
    /// Open the folder and, when the host has never listed it, ask.
    Listing {
        source: KbSourceId,
        path: String,
    },
    /// Nothing to ask: the folder was already listed, or a source was collapsed.
    Toggled,
    Open(KbDocKey),
    /// The source is not readable yet — a clone that has not run, or one that failed. A press
    /// retries it rather than doing nothing.
    Sync(KbSourceId),
}

// ── The right-click menu ────────────────────────────────────────────────────

/// Which of the three kinds of row a menu was raised on. A copy of [`KbRowKind`]'s shape without
/// its payload: the menu is remembered after the tree has moved on, and what it needs to decide
/// what to offer is the kind, not the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KbMenuRow {
    /// A source's own row, whose `rel_path` is empty.
    Root,
    Dir,
    File,
}

/// One command the KB explorer's right-click offers.
///
/// Nothing here is ever drawn disabled: an entry a row cannot do is left out. The explorer greys
/// Paste because a user can *make* there be something to paste; nothing on this menu has that
/// shape — a folder source has no latest version to fetch and no state the user could reach to
/// give it one, so offering it greyed would be a promise with nothing behind it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KbAction {
    /// `RevealKbPath` — the platform's file manager, on the machine the *host* runs on.
    OpenInSystem,
    /// `SyncKbSource`. Offered only for a source that is fetched, which is only a repository.
    Sync,
    /// `AskKbPath`, whose answer goes to the clipboard.
    CopyPath,
    NewFile,
    NewFolder,
    /// The *source's* display name, committed through the whole-list `SetKbSources` write. Not
    /// [`KbAction::Rename`]: a source's name is Ubiq's own label for it, so it is renameable even
    /// where the material behind it is read-only.
    RenameSource,
    /// One entry's name on disk — `RenameKbEntry`, and only where the source is writable.
    Rename,
    Delete,
    /// The line between two groups. An action so that it occupies a slot in the list the pick
    /// indexes into, [`crate::state::explorer::ExplorerAction::Separator`]'s reason.
    Separator,
}

impl KbAction {
    /// What the row says. A separator says nothing: it is drawn as a hairline.
    pub fn label(self) -> &'static str {
        match self {
            KbAction::OpenInSystem => crate::state::explorer::open_in_system_label(),
            // "Get latest version" rather than "Sync": what the user gets is the repository as it
            // is now, and nothing of theirs is being sent anywhere.
            KbAction::Sync => "Get latest version",
            KbAction::CopyPath => "Copy path",
            KbAction::NewFile => "New file",
            KbAction::NewFolder => "New folder",
            // Said as "Rename source" so it cannot be read as renaming the folder behind it.
            KbAction::RenameSource => "Rename source",
            KbAction::Rename => "Rename",
            KbAction::Delete => "Delete",
            KbAction::Separator => "",
        }
    }

    pub fn is_separator(self) -> bool {
        self == KbAction::Separator
    }
}

/// The menu a right-click raised, until it is dismissed or another takes its place.
///
/// Everything the entry list depends on is copied in as the menu opens, so [`KbMenu::entries`]
/// stays a pure function of what was remembered — which is what keeps the pick, an index into that
/// list, pointing at the row that was actually drawn.
#[derive(Clone, Debug)]
pub struct KbMenu {
    pub epoch: u64,
    pub source: KbSourceId,
    /// Empty for the source's own row.
    pub path: String,
    pub row: KbMenuRow,
    /// [`KbSource::is_writable`] for the source this row belongs to.
    pub writable: bool,
    /// Whether the source has to be fetched — [`KbOrigin::is_fetched`], true only for a
    /// repository.
    pub fetched: bool,
    pub x: f32,
    pub y: f32,
}

impl KbMenu {
    pub fn entries(&self) -> Vec<KbAction> {
        kb_menu_entries(self.row, self.writable, self.fetched)
    }
}

/// What a right-click on this row offers, in the order the menu draws it.
///
/// The order is what the pick reads, so whether a row is offered is decided here rather than by
/// whoever draws the list — and a separator occupies a slot of its own for the same reason.
pub fn kb_menu_entries(row: KbMenuRow, writable: bool, fetched: bool) -> Vec<KbAction> {
    // Whole groups, so one that has nothing in it takes its separator with it.
    let groups: Vec<Vec<KbAction>> = match row {
        KbMenuRow::Root => vec![
            match fetched {
                true => vec![KbAction::OpenInSystem, KbAction::Sync, KbAction::CopyPath],
                false => vec![KbAction::OpenInSystem, KbAction::CopyPath],
            },
            match writable {
                true => vec![KbAction::NewFile, KbAction::NewFolder],
                false => Vec::new(),
            },
            // Always offered: renaming a source renames Ubiq's label for it, which is Ubiq's to
            // change whatever the access to the material behind it is.
            vec![KbAction::RenameSource],
        ],
        KbMenuRow::Dir => vec![
            vec![KbAction::OpenInSystem, KbAction::CopyPath],
            match writable {
                true => vec![KbAction::NewFile, KbAction::NewFolder],
                false => Vec::new(),
            },
            match writable {
                true => vec![KbAction::Rename, KbAction::Delete],
                false => Vec::new(),
            },
        ],
        KbMenuRow::File => vec![
            vec![KbAction::OpenInSystem, KbAction::CopyPath],
            match writable {
                true => vec![KbAction::Rename, KbAction::Delete],
                false => Vec::new(),
            },
        ],
    };

    let mut items: Vec<KbAction> = Vec::new();
    for group in groups.into_iter().filter(|group| !group.is_empty()) {
        if !items.is_empty() {
            items.push(KbAction::Separator);
        }
        items.extend(group);
    }
    items
}

impl KbState {
    /// Raise the menu at the pointer, remembering enough of the row to draw it after the tree has
    /// moved on. Answers false for a row the tree no longer holds, which is a menu with nothing
    /// behind it rather than one offering the wrong things.
    pub fn open_menu(&mut self, source: KbSourceId, path: &str, x: f32, y: f32) -> bool {
        let Some(view) = self.source_mut(source) else {
            return false;
        };
        let writable = view.status.source.is_writable();
        let fetched = view.status.source.origin.is_fetched();
        let row = if path.is_empty() {
            KbMenuRow::Root
        } else {
            match node_mut(cow(&mut view.children), path) {
                Some(node) => match node.kind {
                    NodeKind::Dir { .. } => KbMenuRow::Dir,
                    NodeKind::File => KbMenuRow::File,
                },
                None => return false,
            }
        };
        self.menu_epoch = self.menu_epoch.wrapping_add(1);
        self.menu = Some(KbMenu {
            epoch: self.menu_epoch,
            source,
            path: path.to_string(),
            row,
            writable,
            fetched,
            x,
            y,
        });
        true
    }

    /// Take the menu away, if the menu that is up is still the one being dismissed.
    pub fn close_menu(&mut self, epoch: u64) {
        if self.menu.as_ref().is_some_and(|menu| menu.epoch == epoch) {
            self.menu = None;
        }
    }

    /// Forget that a directory was ever listed, so the next listing for it is taken as fresh
    /// rather than merged against a tree the host has since changed under.
    ///
    /// Answers false for a source or a folder the tree does not hold — a `KbChanged` naming a
    /// folder nobody has opened is nothing to re-ask for.
    pub fn mark_unlisted(&mut self, source: KbSourceId, path: &str) -> bool {
        let Some(view) = self.source_mut(source) else {
            return false;
        };
        // The source's own top level is the common case for a create at the root, and it has no
        // node of its own — the row is synthesised from the status beside it.
        if path.is_empty() {
            view.listed = false;
            return true;
        }
        match node_mut(cow(&mut view.children), path) {
            Some(node) => match &mut node.kind {
                NodeKind::Dir { listed, .. } => {
                    *listed = false;
                    true
                }
                NodeKind::File => false,
            },
            None => false,
        }
    }

    /// Take the last refusal this source saw. Written by a failed listing, and by a failed write,
    /// create, rename or delete that named no document on screen.
    pub fn set_error(&mut self, source: KbSourceId, error: Option<String>) {
        if let Some(view) = self.source_mut(source) {
            view.error = error;
        }
    }

    /// Take the configuration the host answered with.
    ///
    /// Sources already held keep their tree, so a settings edit that renamed one does not shut
    /// every folder the user had open. One that has gone goes with its tree; one that is new
    /// arrives open and unlisted, because a knowledge base with one source should not need a
    /// click to show anything.
    pub fn accept(&mut self, sources: Vec<KbSourceStatus>) {
        let mut held: Vec<KbSourceView> = std::mem::take(&mut self.sources);
        self.sources = sources
            .into_iter()
            .map(|status| {
                match held.iter().position(|view| view.id() == status.source.id) {
                    Some(at) => {
                        let mut view = held.remove(at);
                        // A filter edit changes what belongs in the tree, so the tree it had is
                        // no longer an answer to the question the source now asks.
                        if view.status.source.filter != status.source.filter {
                            view.children = Arc::new(Vec::new());
                            view.listed = false;
                        }
                        view.status = status;
                        view
                    }
                    None => KbSourceView::new(status),
                }
            })
            .collect();
        self.loaded = true;
        // A selection into a source that has gone is not a selection, and nor is a tab open on
        // one: a document whose source was removed names nothing to read or write.
        if let Some(key) = &self.selected
            && !self.sources.iter().any(|view| view.id() == key.source)
        {
            self.selected = None;
        }
        let sources: Vec<KbSourceId> = self.sources.iter().map(|view| view.id()).collect();
        self.docs
            .retain(|doc| doc.kb_source.is_some_and(|id| sources.contains(&id)));
    }

    /// One source's state moved on its own — a clone started, got somewhere, finished or failed.
    ///
    /// A source that has just become ready drops what it had listed: it was listed against an
    /// empty folder, and the clone has since filled it.
    pub fn source_changed(&mut self, source: KbSourceId, state: KbSourceState) {
        let Some(view) = self.source_mut(source) else {
            return;
        };
        let became_ready = !view.status.state.is_ready() && state.is_ready();
        view.status.state = state;
        if became_ready {
            view.children = Arc::new(Vec::new());
            view.listed = false;
            view.loading = false;
        }
    }

    /// One open document by its tab key. What a KB panel draws and what its tab reports are both
    /// this — `AppState::file`'s own shape, said for the documents half.
    pub fn doc(&self, key: &str) -> Option<&OpenFile> {
        self.docs.iter().find(|file| file.key() == key)
    }

    pub fn doc_mut(&mut self, key: &str) -> Option<&mut OpenFile> {
        self.docs.iter_mut().find(|file| file.key() == key)
    }

    /// Put a document in front of the user before its bytes exist, answering whether this call is
    /// what opened it. One already open is left exactly as it is, so a second click cannot open it
    /// twice or discard what has been typed into it.
    pub fn open_doc(&mut self, key: &KbDocKey, markdown_open: super::editor::ViewLayout) -> bool {
        if self.doc(&key.tab_key()).is_some() {
            return false;
        }
        self.docs
            .push(OpenFile::kb_opening(key.source, &key.path, markdown_open));
        true
    }

    /// Take a document's tab away. Answers whether there was one.
    pub fn close_doc(&mut self, key: &str) -> bool {
        let Some(at) = self.docs.iter().position(|file| file.key() == key) else {
            return false;
        };
        self.docs.remove(at);
        true
    }

    pub fn source(&self, source: KbSourceId) -> Option<&KbSourceView> {
        self.sources.iter().find(|view| view.id() == source)
    }

    pub fn source_mut(&mut self, source: KbSourceId) -> Option<&mut KbSourceView> {
        self.sources.iter_mut().find(|view| view.id() == source)
    }

    /// Whether a knowledge-base address is something this window can currently show — on
    /// [`super::explorer::locate`]'s own three-way reasoning, over this source's own lazily-listed
    /// tree.
    ///
    /// `Presence::Dead` for the source itself only once `self.loaded` is true: a project whose
    /// configuration has not answered yet has an empty `sources`, and that must read the same as
    /// "not knowable" rather than as every source in it having been removed.
    pub fn presence(&self, source: KbSourceId, path: &str) -> crate::state::explorer::Presence {
        let Some(view) = self.source(source) else {
            return if self.loaded {
                crate::state::explorer::Presence::Dead
            } else {
                crate::state::explorer::Presence::Unknown
            };
        };
        super::explorer::locate(&view.children, view.listed, path)
    }

    /// The sources as the settings form edits them.
    pub fn configured(&self) -> Vec<KbSource> {
        self.sources
            .iter()
            .map(|view| view.status.source.clone())
            .collect()
    }

    /// Take a listing. Answers false when it names a folder the tree does not hold — a folder
    /// collapsed away while its listing was in flight — on `ExplorerState::merge`'s discipline,
    /// and entries are matched by name for the same reason: a re-listing must not lose the
    /// folders the user had open.
    pub fn merge(&mut self, source: KbSourceId, listing: DirListing) -> bool {
        let Some(view) = self.source_mut(source) else {
            return false;
        };
        if listing.rel_path.is_empty() {
            merge_children(cow(&mut view.children), listing.entries);
            view.listed = true;
            view.loading = false;
            view.truncated = listing.truncated;
            view.error = None;
            return true;
        }
        let Some(node) = node_mut(cow(&mut view.children), &listing.rel_path) else {
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
        merge_children(cow(children), listing.entries);
        *listed = true;
        *loading = false;
        *truncated = listing.truncated;
        true
    }

    /// Mark a folder as waiting for its listing, so the row can say so.
    pub fn mark_loading(&mut self, source: KbSourceId, path: &str) {
        let Some(view) = self.source_mut(source) else {
            return;
        };
        if path.is_empty() {
            view.loading = true;
            return;
        }
        if let Some(node) = node_mut(cow(&mut view.children), path)
            && let NodeKind::Dir { loading, .. } = &mut node.kind
        {
            *loading = true;
        }
    }

    /// A press on a row, and what it asks for.
    pub fn click(&mut self, source: KbSourceId, path: &str) -> KbPressed {
        let Some(view) = self.source_mut(source) else {
            return KbPressed::Toggled;
        };
        if path.is_empty() {
            if !view.status.state.is_ready() {
                return KbPressed::Sync(source);
            }
            view.expanded = !view.expanded;
            if view.expanded && !view.listed {
                return KbPressed::Listing {
                    source,
                    path: String::new(),
                };
            }
            return KbPressed::Toggled;
        }
        let Some(node) = node_mut(cow(&mut view.children), path) else {
            return KbPressed::Toggled;
        };
        match &mut node.kind {
            NodeKind::Dir {
                expanded, listed, ..
            } => {
                *expanded = !*expanded;
                let ask = *expanded && !*listed;
                if ask {
                    KbPressed::Listing {
                        source,
                        path: path.to_string(),
                    }
                } else {
                    KbPressed::Toggled
                }
            }
            NodeKind::File => KbPressed::Open(KbDocKey::new(source, path)),
        }
    }

    /// Every row the panel draws, sources and their open folders flattened in order.
    pub fn rows(&self) -> Vec<KbRow> {
        let mut rows = Vec::new();
        for view in &self.sources {
            let id = view.id();
            rows.push(KbRow {
                source: id,
                path: String::new(),
                name: view.name().to_string(),
                depth: 0,
                kind: KbRowKind::Root {
                    expanded: view.expanded,
                    state: view.status.state.clone(),
                    origin: view.origin(),
                    error: view.error.clone(),
                },
                selected: false,
            });
            if view.expanded {
                self.push_rows(id, &view.children, 1, &mut rows);
            }
        }
        rows
    }

    fn push_rows(
        &self,
        source: KbSourceId,
        nodes: &[FileNode],
        depth: usize,
        rows: &mut Vec<KbRow>,
    ) {
        for node in nodes {
            let selected = self
                .selected
                .as_ref()
                .is_some_and(|key| key.source == source && key.path == node.path);
            match &node.kind {
                NodeKind::Dir {
                    children,
                    expanded,
                    loading,
                    ..
                } => {
                    rows.push(KbRow {
                        source,
                        path: node.path.clone(),
                        name: node.name.clone(),
                        depth,
                        kind: KbRowKind::Dir {
                            expanded: *expanded,
                            loading: *loading,
                        },
                        selected,
                    });
                    if *expanded {
                        self.push_rows(source, children, depth + 1, rows);
                    }
                }
                NodeKind::File => rows.push(KbRow {
                    source,
                    path: node.path.clone(),
                    name: node.name.clone(),
                    depth,
                    kind: KbRowKind::File,
                    selected,
                }),
            }
        }
    }
}

/// `Arc::make_mut` spelled once. The tree is shared with whatever snapshot is on the frame, and a
/// write clones only the level it touches.
fn cow(children: &mut Arc<Vec<FileNode>>) -> &mut Vec<FileNode> {
    Arc::make_mut(children)
}

/// The node at a path, if the tree holds it. Walks by prefix rather than by split, so a name
/// containing a separator cannot be reached by a path that does not name it.
fn node_mut<'a>(nodes: &'a mut Vec<FileNode>, path: &str) -> Option<&'a mut FileNode> {
    let mut current = nodes;
    loop {
        let at = current.iter().position(|node| {
            node.path == path
                || (path.starts_with(&node.path) && path[node.path.len()..].starts_with('/'))
        })?;
        if current[at].path == path {
            return Some(&mut current[at]);
        }
        let NodeKind::Dir { children, .. } = &mut current[at].kind else {
            return None;
        };
        current = cow(children);
    }
}

/// Merge one level, matching by name: what is still there keeps its subtree and its open flag,
/// what has gone goes with its subtree, what is new arrives shut and unlisted.
fn merge_children(into: &mut Vec<FileNode>, entries: Vec<ubiq_proto::files::DirEntry>) {
    let held = std::mem::take(into);
    for entry in entries {
        let previous = held.iter().find(|node| node.name == entry.name);
        let kind = match entry.kind {
            EntryKind::Dir => match previous.map(|node| &node.kind) {
                Some(NodeKind::Dir {
                    children,
                    expanded,
                    listed,
                    loading,
                    truncated,
                }) => NodeKind::Dir {
                    children: children.clone(),
                    expanded: *expanded,
                    listed: *listed,
                    loading: *loading,
                    truncated: *truncated,
                },
                _ => NodeKind::Dir {
                    children: Arc::new(Vec::new()),
                    expanded: false,
                    listed: false,
                    loading: false,
                    truncated: false,
                },
            },
            _ => NodeKind::File,
        };
        into.push(FileNode {
            name: entry.name,
            path: entry.rel_path,
            kind,
            size: entry.size,
            git: None,
            repo: None,
            readable: true,
        });
    }
}

// ── The "Add source" form ───────────────────────────────────────────────────

/// Which of the three things a new source is. Asked before anything else, because every row under
/// it is a question only one kind has — and the kinds are kept apart from [`KbOrigin`] because a
/// half-filled form has a kind long before it has an origin to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KbKind {
    Folder,
    Git,
    /// [`KbOrigin::Internal`] — a wiki Ubiq keeps itself, with nothing to point at and nothing to
    /// fetch.
    Wiki,
}

impl KbKind {
    pub fn label(self) -> &'static str {
        match self {
            KbKind::Folder => "Folder",
            KbKind::Git => "Git repository",
            KbKind::Wiki => "Wiki",
        }
    }
}

/// Which of the form's lists is down. One at a time — the window's rule — held on the form rather
/// than in `open_menu` for [`super::new_agent::OpenList`]'s reason: the modal is redrawn from state
/// on every frame, and a picker inside one has nowhere else to keep it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KbList {
    Kind,
    Branch,
    Store,
    Access,
}

/// How far the Check button has got against the typed URL.
///
/// Typed rather than three loose fields, on [`KbSourceState`]'s own reasoning: "checking",
/// "twelve branches" and "that credential was refused" need different sentences, and only one of
/// them can be true at a time.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum KbUrlCheck {
    /// Nothing asked yet, or the URL has been edited since the last answer.
    #[default]
    Idle,
    Checking,
    /// The host reached the repository and listed this many branches.
    Checked(usize),
    /// The sentence to print under the field.
    Failed(String),
}

/// The "Add source" modal while it is up: every answer, and nothing about how it is drawn.
///
/// **A kind switch clears what the other kind filled.** A URL left behind by a repository the user
/// changed their mind about would otherwise ride back in the moment they changed it back, and a
/// form that remembers an answer it stopped showing is one nobody can read off the screen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KbSourceForm {
    /// `None` is nothing chosen: every row below the kind is drawn inert until this is answered,
    /// the discipline `crate::ui::new_agent` states for its own first row.
    pub kind: Option<KbKind>,
    pub name: String,
    /// Whether the user has typed into the name field. Once true the name is never re-seeded:
    /// a deliberate name overwritten by a path the user picked afterwards is a rename nobody asked
    /// for.
    pub named: bool,
    /// The folder chosen through Ubiq's own picker, absolute on the machine the *host* runs on.
    /// The one path this form holds, and it holds it for the reason `KbOrigin::Folder` does: the
    /// row has to print it back.
    pub path: Option<String>,
    pub url: String,
    pub check: KbUrlCheck,
    /// The branch listing this form is waiting on. An answer naming any other id is discarded —
    /// `state::clone::CloneState`'s discipline, and the reason Check can be pressed twice.
    pub branches_query: Option<RepoQueryId>,
    pub branches: Vec<String>,
    /// `None` is the repository's own default, which is what [`KbOrigin::Git`] means by an absent
    /// branch.
    pub branch: Option<String>,
    pub store: KbStore,
    pub access: KbAccess,
    pub open: Option<KbList>,
}

impl KbSourceForm {
    /// Choose what is being added, forgetting whatever the other kind had filled in.
    pub fn set_kind(&mut self, kind: KbKind) {
        if self.kind == Some(kind) {
            return;
        }
        self.kind = Some(kind);
        self.path = None;
        self.url = String::new();
        self.check = KbUrlCheck::Idle;
        self.branches_query = None;
        self.branches.clear();
        self.branch = None;
        self.store = KbStore::default();
        self.seed_name();
    }

    /// The folder the picker answered with.
    pub fn set_path(&mut self, path: String) {
        self.path = Some(path);
        self.seed_name();
    }

    /// The URL field changed. Whatever was checked is no longer about what is typed, so the
    /// branches go with it — a branch list belonging to the previous URL is worse than none.
    pub fn set_url(&mut self, url: String) {
        if self.url == url {
            return;
        }
        self.url = url;
        self.check = KbUrlCheck::Idle;
        self.branches_query = None;
        self.branches.clear();
        self.branch = None;
        self.seed_name();
    }

    /// What the name field says, and that the user said it.
    pub fn rename(&mut self, name: String) {
        self.named = true;
        self.name = name;
    }

    /// Fill the name from whatever the form points at, until the user names it themselves.
    pub fn seed_name(&mut self) {
        if self.named {
            return;
        }
        self.name = self
            .provisional_origin()
            .map(|origin| origin.default_name())
            .unwrap_or_default();
    }

    /// What the form points at as it stands — which is not yet what it would be *saved* as: a URL
    /// nobody has checked still names a repository well enough to take a name from.
    fn provisional_origin(&self) -> Option<KbOrigin> {
        match self.kind? {
            KbKind::Folder => self.path.clone().map(|path| KbOrigin::Folder { path }),
            KbKind::Git => {
                let url = self.url.trim();
                (!url.is_empty()).then(|| KbOrigin::Git {
                    url: url.to_string(),
                    branch: self.branch.clone(),
                    store: self.store,
                })
            }
            KbKind::Wiki => Some(KbOrigin::Internal),
        }
    }

    /// What may be written to this source. A wiki is always read-write, whatever the row shows:
    /// there is nothing fetched or user-picked under it for read-only to protect.
    pub fn access(&self) -> KbAccess {
        match self.kind {
            Some(KbKind::Wiki) => KbAccess::ReadWrite,
            _ => self.access,
        }
    }

    /// Whether the form has been answered: a kind, a name, and the one thing that kind needs — a
    /// folder, or a URL the host has actually reached.
    pub fn is_answerable(&self) -> bool {
        if self.name.trim().is_empty() {
            return false;
        }
        match self.kind {
            None => false,
            Some(KbKind::Folder) => self.path.as_deref().is_some_and(|path| !path.is_empty()),
            Some(KbKind::Git) => matches!(self.check, KbUrlCheck::Checked(_)),
            Some(KbKind::Wiki) => true,
        }
    }

    /// The source this form would append, or nothing while it is not answerable.
    pub fn as_source(&self) -> Option<KbSource> {
        if !self.is_answerable() {
            return None;
        }
        Some(KbSource {
            id: KbSourceId::generate(),
            name: self.name.trim().to_string(),
            origin: self.provisional_origin()?,
            filter: String::new(),
            access: self.access(),
        })
    }

    /// A branch listing arrived. `false` means it named a query this form no longer holds, and
    /// nothing was applied.
    ///
    /// The preselection is the repository's reported default, then `main`, then `master`, and
    /// otherwise nothing at all — which [`KbOrigin::Git`] reads as "whatever the repository's
    /// default is", the honest answer when none of the three is on offer.
    pub fn accept_branches(
        &mut self,
        query_id: RepoQueryId,
        branches: Vec<String>,
        default: Option<String>,
    ) -> bool {
        if self.branches_query != Some(query_id) {
            return false;
        }
        self.branches_query = None;
        self.check = KbUrlCheck::Checked(branches.len());
        self.branch = default
            .filter(|branch| branches.contains(branch))
            .or_else(|| branches.iter().find(|it| *it == "main").cloned())
            .or_else(|| branches.iter().find(|it| *it == "master").cloned());
        self.branches = branches;
        true
    }

    /// The check failed. Same guard, same answer.
    pub fn accept_error(&mut self, query_id: RepoQueryId, error: &CloneError) -> bool {
        if self.branches_query != Some(query_id) {
            return false;
        }
        self.branches_query = None;
        self.check = KbUrlCheck::Failed(super::clone::clone_error_note(error));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::files::DirEntry;

    fn status(name: &str) -> KbSourceStatus {
        KbSourceStatus {
            source: KbSource {
                id: KbSourceId::generate(),
                name: name.into(),
                origin: KbOrigin::Folder {
                    path: format!("/{name}"),
                },
                filter: String::new(),
                access: Default::default(),
            },
            state: KbSourceState::Ready,
        }
    }

    fn listing(rel_path: &str, entries: &[(&str, EntryKind)]) -> DirListing {
        DirListing {
            rel_path: rel_path.into(),
            entries: entries
                .iter()
                .map(|(name, kind)| DirEntry {
                    name: (*name).into(),
                    rel_path: if rel_path.is_empty() {
                        (*name).to_string()
                    } else {
                        format!("{rel_path}/{name}")
                    },
                    kind: *kind,
                    size: None,
                    symlink: false,
                })
                .collect(),
            truncated: false,
        }
    }

    #[test]
    fn a_reload_keeps_the_trees_of_the_sources_that_stayed() {
        let mut kb = KbState::default();
        let one = status("docs");
        let id = one.source.id;
        kb.accept(vec![one.clone()]);
        assert!(kb.merge(id, listing("", &[("guide.md", EntryKind::File)])));
        let mut renamed = one;
        renamed.source.name = "Documents".into();
        kb.accept(vec![renamed]);
        assert_eq!(kb.sources.len(), 1);
        assert_eq!(kb.sources[0].name(), "Documents");
        assert_eq!(kb.sources[0].children.len(), 1);
    }

    #[test]
    fn a_filter_edit_drops_the_tree_it_answered() {
        let mut kb = KbState::default();
        let one = status("docs");
        let id = one.source.id;
        kb.accept(vec![one.clone()]);
        kb.merge(id, listing("", &[("guide.md", EntryKind::File)]));
        let mut filtered = one;
        filtered.source.filter = "*.md".into();
        kb.accept(vec![filtered]);
        assert!(kb.sources[0].children.is_empty());
        assert!(!kb.sources[0].listed);
    }

    #[test]
    fn a_click_on_a_shut_folder_asks_for_its_listing_once() {
        let mut kb = KbState::default();
        let one = status("docs");
        let id = one.source.id;
        kb.accept(vec![one]);
        kb.merge(id, listing("", &[("notes", EntryKind::Dir)]));
        assert!(matches!(
            kb.click(id, "notes"),
            KbPressed::Listing { path, .. } if path == "notes"
        ));
        kb.merge(id, listing("notes", &[("a.md", EntryKind::File)]));
        // Shut, then open again: the listing is held, so nothing is asked for twice.
        assert!(matches!(kb.click(id, "notes"), KbPressed::Toggled));
        assert!(matches!(kb.click(id, "notes"), KbPressed::Toggled));
        assert_eq!(kb.rows().len(), 3);
    }

    #[test]
    fn a_click_on_a_file_opens_it() {
        let mut kb = KbState::default();
        let one = status("docs");
        let id = one.source.id;
        kb.accept(vec![one]);
        kb.merge(id, listing("", &[("guide.md", EntryKind::File)]));
        let KbPressed::Open(key) = kb.click(id, "guide.md") else {
            panic!("a file opens");
        };
        assert_eq!(key.name(), "guide.md");
    }

    #[test]
    fn a_source_that_is_not_ready_syncs_rather_than_opens() {
        let mut kb = KbState::default();
        let mut one = status("wiki");
        one.state = KbSourceState::Pending;
        let id = one.source.id;
        kb.accept(vec![one]);
        assert!(matches!(kb.click(id, ""), KbPressed::Sync(_)));
    }

    #[test]
    fn a_source_that_becomes_ready_forgets_what_it_listed_while_empty() {
        let mut kb = KbState::default();
        let mut one = status("wiki");
        one.state = KbSourceState::Pending;
        let id = one.source.id;
        kb.accept(vec![one]);
        kb.merge(id, listing("", &[]));
        kb.source_changed(id, KbSourceState::Ready);
        assert!(!kb.sources[0].listed);
    }

    // ── presence — a task attachment's dead chip (T-84) ────────────────────

    /// A document a listed source actually holds reads as present.
    #[test]
    fn presence_is_live_for_a_document_the_source_holds() {
        let mut kb = KbState::default();
        let one = status("docs");
        let id = one.source.id;
        kb.accept(vec![one]);
        kb.merge(id, listing("", &[("guide.md", EntryKind::File)]));
        assert_eq!(kb.presence(id, "guide.md"), crate::state::explorer::Presence::Live);
    }

    /// A name a listed source's tree does not hold is gone.
    #[test]
    fn presence_is_dead_for_a_name_a_listed_source_does_not_hold() {
        let mut kb = KbState::default();
        let one = status("docs");
        let id = one.source.id;
        kb.accept(vec![one]);
        kb.merge(id, listing("", &[("guide.md", EntryKind::File)]));
        assert_eq!(
            kb.presence(id, "never-existed.md"),
            crate::state::explorer::Presence::Dead
        );
    }

    /// The source itself is gone — removed from the project's configuration — once the
    /// configuration has actually answered and no longer names it. That is a real dead source,
    /// not an unlisted folder.
    #[test]
    fn presence_is_dead_for_a_source_the_configuration_no_longer_names() {
        let mut kb = KbState::default();
        let one = status("docs");
        let id = one.source.id;
        kb.accept(vec![one]);
        kb.accept(vec![]);
        assert_eq!(
            kb.presence(id, "guide.md"),
            crate::state::explorer::Presence::Dead
        );
    }

    /// Before the configuration has answered at all, an empty `sources` must not read as every
    /// source having been removed — that is "not knowable yet", the same as an unlisted folder.
    #[test]
    fn presence_is_unknown_before_the_configuration_has_answered() {
        let kb = KbState::default();
        assert_eq!(
            kb.presence(KbSourceId::generate(), "guide.md"),
            crate::state::explorer::Presence::Unknown
        );
    }

    /// A path below a folder the source has not listed yet is not knowable, and must not read as
    /// dead just because this window has not looked.
    #[test]
    fn presence_is_unknown_below_a_folder_the_source_has_not_listed() {
        let mut kb = KbState::default();
        let one = status("docs");
        let id = one.source.id;
        kb.accept(vec![one]);
        kb.merge(id, listing("", &[("notes", EntryKind::Dir)]));
        assert_eq!(
            kb.presence(id, "notes/a.md"),
            crate::state::explorer::Presence::Unknown
        );
    }
}
