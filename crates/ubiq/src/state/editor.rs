//! The files open in the centre pane.
//!
//! **Each open file owns its buffer.** That is what makes dirty a comparison rather than a flag
//! somebody has to remember to set: the baseline is exactly the bytes the host sent, and the buffer
//! is what the user has typed since. It is also what lets a file be open in a project the window is
//! not currently pointed at, and lets a tab exist before its bytes do.
//!
//! So this module names the component library's editor, which the tab strip's own state cannot
//! avoid once the buffer *is* the file's state. The mapping from a language onto the highlighter's
//! own enum still lives in `ui/editor.rs`, because that is a drawing decision and this is not.

use std::ops::Range;

use gpui::{Entity, Pixels, Point, Subscription};
use gpui_component::input::EditorState;
use ubiq_proto::files::{DiffBase, FileDiff, FileVersion};

/// The languages the editor highlights. Anything else opens as plain text, which is the general
/// case rather than a fallback.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileLanguage {
    Tsx,
    TypeScript,
    JavaScript,
    Json,
    Rust,
    Markdown,
    Yaml,
    Java,
    Python,
    Go,
    CSharp,
    Bash,
    C,
    Swift,
    Css,
    Html,
    Sql,
    Toml,
    Kotlin,
    Diff,
    Plain,
}

impl FileLanguage {
    /// What the status bar calls it.
    pub fn label(self) -> &'static str {
        match self {
            FileLanguage::Tsx | FileLanguage::TypeScript => "TypeScript",
            FileLanguage::JavaScript => "JavaScript",
            FileLanguage::Json => "JSON",
            FileLanguage::Rust => "Rust",
            FileLanguage::Markdown => "Markdown",
            FileLanguage::Yaml => "YAML",
            FileLanguage::Java => "Java",
            FileLanguage::Python => "Python",
            FileLanguage::Go => "Go",
            FileLanguage::CSharp => "C#",
            FileLanguage::Bash => "Shell",
            FileLanguage::C => "C",
            FileLanguage::Swift => "Swift",
            FileLanguage::Css => "CSS",
            FileLanguage::Html => "HTML",
            FileLanguage::Sql => "SQL",
            FileLanguage::Toml => "TOML",
            FileLanguage::Kotlin => "Kotlin",
            FileLanguage::Diff => "Diff",
            FileLanguage::Plain => "Plain Text",
        }
    }

    /// The language a path's extension names.
    ///
    /// Only extensions the highlighter actually has a grammar for are named; everything else is
    /// plain text rather than highlighted as something it is not.
    pub fn of(path: &str) -> Self {
        match extension(path).as_str() {
            "tsx" => FileLanguage::Tsx,
            "ts" | "mts" | "cts" => FileLanguage::TypeScript,
            "js" | "mjs" | "cjs" | "jsx" => FileLanguage::JavaScript,
            "json" | "jsonc" => FileLanguage::Json,
            "rs" => FileLanguage::Rust,
            "md" | "markdown" => FileLanguage::Markdown,
            "yaml" | "yml" => FileLanguage::Yaml,
            "java" => FileLanguage::Java,
            "py" | "pyw" | "pyi" => FileLanguage::Python,
            "go" => FileLanguage::Go,
            "cs" | "csx" => FileLanguage::CSharp,
            "sh" | "bash" | "zsh" | "ksh" => FileLanguage::Bash,
            "c" | "h" => FileLanguage::C,
            "swift" => FileLanguage::Swift,
            "css" => FileLanguage::Css,
            "html" | "htm" => FileLanguage::Html,
            "sql" => FileLanguage::Sql,
            "toml" => FileLanguage::Toml,
            "kt" | "kts" => FileLanguage::Kotlin,
            "diff" | "patch" => FileLanguage::Diff,
            _ => FileLanguage::Plain,
        }
    }
}

/// What draws a file, once its bytes are here.
///
/// The path's extension picks one, and anything unrecognised is the editor — the general case
/// rather than a fallback. A viewer is a pure function of bytes and a kind: it opens no file and
/// resolves no path, and where the bytes came from is not its business.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewerKind {
    /// The text, highlighted. Every extension with no viewer of its own lands here.
    Editor,
    /// The source, the rendered document, or both.
    Markdown,
    /// A diagram the host rendered, drawn from the image it sent back.
    Mermaid,
    /// A scene drawn natively from its own JSON.
    Excalidraw,
    /// The image itself.
    Image,
}

impl ViewerKind {
    /// The viewer a path's extension names.
    pub fn of(path: &str) -> Self {
        match extension(path).as_str() {
            "md" | "markdown" => ViewerKind::Markdown,
            "mmd" | "mermaid" => ViewerKind::Mermaid,
            "excalidraw" => ViewerKind::Excalidraw,
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "tif" | "tiff" | "ico" => {
                ViewerKind::Image
            }
            _ => ViewerKind::Editor,
        }
    }

    /// Whether the viewer has a source to show beside what it drew. The editor is only ever
    /// source, an image has none at all, and an Excalidraw scene is preview-only: its source is a
    /// serialised document nobody edits by hand, so there is no source layout to turn to.
    pub fn has_preview(self) -> bool {
        matches!(self, ViewerKind::Markdown | ViewerKind::Mermaid)
    }
}

/// Which of a viewer's layouts is on screen. The one piece of per-tab state a viewer keeps, and
/// what it writes into the dock's saved layout so a document reopens as it was left.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewLayout {
    /// The bytes, in the editor.
    Source,
    /// What the viewer drew, alone.
    #[default]
    Preview,
    /// Both, side by side.
    Split,
}

impl ViewLayout {
    /// What the header's toggle calls it.
    pub fn label(self) -> &'static str {
        match self {
            ViewLayout::Source => "Source",
            ViewLayout::Preview => "Preview",
            ViewLayout::Split => "Split",
        }
    }

    /// The three, in the order the toggle draws them.
    pub fn all() -> [ViewLayout; 3] {
        [ViewLayout::Source, ViewLayout::Preview, ViewLayout::Split]
    }

    /// Whether the source half is drawn in this layout.
    pub fn shows_source(self) -> bool {
        matches!(self, ViewLayout::Source | ViewLayout::Split)
    }

    /// Whether the drawn half is drawn in this layout.
    pub fn shows_preview(self) -> bool {
        matches!(self, ViewLayout::Preview | ViewLayout::Split)
    }
}

/// What a tab is looking at: the file itself, or a comparison the host computed from it.
///
/// A diff is not a file, which is what makes it a subject rather than a viewer kind. Its content
/// is not on disk anywhere, so it is asked for with its own message and arrives as rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Subject {
    File,
    Diff(DiffBase),
}

impl Subject {
    /// The prefix that keeps a file and its diff two tabs rather than one.
    fn tag(self) -> &'static str {
        match self {
            Subject::File => "",
            Subject::Diff(DiffBase::Head) => "diff:head:",
            Subject::Diff(DiffBase::Index) => "diff:index:",
        }
    }

    /// What the tab says after the file's name.
    pub fn suffix(self) -> &'static str {
        match self {
            Subject::File => "",
            Subject::Diff(DiffBase::Head) => " · diff",
            Subject::Diff(DiffBase::Index) => " · staged",
        }
    }
}

/// The key that identifies one tab, and the key a saved layout and the view prefs name it by.
///
/// It is the path for a file and the path behind a prefix for a diff, so that opening a file's
/// diff never takes over the tab holding the file.
pub fn tab_key(path: &str, subject: Subject) -> String {
    format!("{}{path}", subject.tag())
}

/// Split a key back into what it names. An unprefixed key is the file itself.
pub fn from_tab_key(key: &str) -> (String, Subject) {
    for subject in [
        Subject::Diff(DiffBase::Head),
        Subject::Diff(DiffBase::Index),
    ] {
        if let Some(path) = key.strip_prefix(subject.tag()) {
            return (path.to_string(), subject);
        }
    }
    (key.to_string(), Subject::File)
}

/// What a tab is showing, which is not always a buffer.
///
/// A tab exists from the click that asked for the file, so that a click has an effect, a second
/// click cannot ask twice, and a read that fails has somewhere to say so.
pub enum FileBody {
    /// Asked for, not yet answered.
    Loading,
    /// What the host sent, and the buffer it is being edited in.
    Text {
        state: Entity<EditorState>,
        /// Exactly the text the host sent, so dirty is a comparison against a fact.
        baseline: String,
        /// A read the host cut short. Readable, never savable: writing a prefix back would shorten
        /// the file.
        truncated: bool,
        /// What to hand back with a save, so a write cannot land on somebody else's change. Absent
        /// when the read was truncated, which is what makes such a buffer unsavable mechanically.
        version: Option<FileVersion>,
    },
    /// The hunks the host computed. A diff has no buffer, because there is nothing to edit: the
    /// comparison is not a file, and no diff library entered the interface to make it.
    Diff(Box<FileDiff>),
    /// Bytes a viewer draws rather than a buffer edits — an image, and nothing else today.
    ///
    /// Kept whole and undecoded, because the thing that draws them is a decoder: turning them into
    /// text first would be lossy in exactly the way that matters.
    Bytes(Vec<u8>),
    /// Bytes the editor will not show.
    Binary,
    /// Why there are none.
    Failed(String),
}

/// Where a save has got to. The text that is in flight travels with it, so the acknowledgement
/// clears dirty against what was written rather than against whatever has been typed since.
pub enum SaveState {
    Idle,
    Saving(String),
    Failed(String),
}

pub struct OpenFile {
    pub name: String,
    /// Project-relative, as every path the interface holds is.
    pub path: String,
    /// The file itself, or a comparison the host made from it.
    pub subject: Subject,
    pub language: FileLanguage,
    /// What draws it. Chosen from the extension once, when the tab opens.
    pub viewer: ViewerKind,
    /// Which of the viewer's layouts is on screen. Meaningless for a viewer with no preview, and
    /// harmless there.
    pub layout: ViewLayout,
    pub body: FileBody,
    pub save: SaveState,
    /// A temporary preview tab: not yet promoted. The first edit or an explicit open makes it
    /// permanent, and opening another temp tab closes the one before it.
    pub temporary: bool,
    /// A file dropped in from outside every open project: read-only, hosted by the active project
    /// rather than its own. Exists so the tab can be drawn differently; `savable` — not this — is
    /// what actually refuses the write.
    pub guest: bool,
    /// A buffer that has never been written anywhere: the tab a new-file keystroke opens. Beside
    /// `guest` and for the same reason — the tab draws differently — and a save on one asks where
    /// to put it rather than sending anything, because the path it carries names nothing on disk.
    pub untitled: bool,
    /// Whether the YAML frontmatter disclosure is open. Per-tab UI state that defaults to closed
    /// so newly opened documents start clean.
    pub frontmatter_open: bool,
    /// Cached rather than compared every frame: a per-frame comparison is the file's length times
    /// the tabs open times the frame rate.
    dirty: bool,
    /// The buffer's change event, which is what keeps `dirty` current. Held here because it must
    /// live exactly as long as the file does.
    _change: Option<Subscription>,
    /// Where the cursor and scroll were when an external change sent this tab back to
    /// [`FileBody::Loading`], so the fresh buffer [`OpenFile::attach`] builds can be put back where
    /// the user was looking rather than opening at the top. Set only for a background tab: the tab
    /// on screen is never silently reloaded in the first place.
    restore: Option<(Range<usize>, Point<Pixels>)>,
}

impl OpenFile {
    /// A tab with no bytes yet: what a click on the explorer produces before the host has answered.
    pub fn pending(path: &str) -> Self {
        Self::pending_on(path, Subject::File)
    }

    /// The same, for a tab looking at something the host will make from the file.
    pub fn pending_on(path: &str, subject: Subject) -> Self {
        Self::opening(path, subject, ViewLayout::default())
    }

    /// Open a tab, taking the markdown default from settings when the path is a markdown file.
    pub fn opening(path: &str, subject: Subject, markdown_open: ViewLayout) -> Self {
        let viewer = ViewerKind::of(path);
        let layout = match viewer {
            ViewerKind::Markdown => markdown_open,
            other if other.has_preview() => ViewLayout::default(),
            _ => ViewLayout::Source,
        };
        Self {
            name: leaf(path).to_string(),
            path: path.to_string(),
            subject,
            language: FileLanguage::of(path),
            viewer,
            layout,
            body: FileBody::Loading,
            save: SaveState::Idle,
            temporary: false,
            guest: false,
            untitled: false,
            frontmatter_open: false,
            dirty: false,
            _change: None,
            restore: None,
        }
    }

    /// A temporary preview tab with no bytes yet.
    pub fn temporary(path: &str, markdown_open: ViewLayout) -> Self {
        Self {
            temporary: true,
            ..Self::opening(path, Subject::File, markdown_open)
        }
    }

    /// A buffer with nowhere to go yet. `path` is the name the tab shows until a save-as gives it
    /// a real one; nothing is read for it, because there is nothing to read.
    pub fn untitled(path: &str, markdown_open: ViewLayout) -> Self {
        Self {
            untitled: true,
            ..Self::opening(path, Subject::File, markdown_open)
        }
    }

    /// Point the tab at another path, keeping the buffer it holds.
    ///
    /// What a rename and a save-as both do: the bytes did not change, so re-reading them would
    /// only risk dropping whatever has been typed since.
    pub fn retarget(&mut self, path: &str) {
        self.name = leaf(path).to_string();
        self.path = path.to_string();
        self.language = FileLanguage::of(path);
        self.viewer = ViewerKind::of(path);
        self.untitled = false;
    }

    /// The key this tab is known by, in the dock's saved layout and in the view prefs.
    pub fn key(&self) -> String {
        tab_key(&self.path, self.subject)
    }

    /// Give the tab the hunks the host computed. A diff replaces nothing and is never edited.
    pub fn attach_diff(&mut self, diff: FileDiff) {
        self.body = FileBody::Diff(Box::new(diff));
        self.save = SaveState::Idle;
        self.dirty = false;
        self._change = None;
    }

    /// Put the viewer into one of its layouts. A viewer with no preview has only its source.
    pub fn set_layout(&mut self, layout: ViewLayout) {
        if self.viewer.has_preview() {
            self.layout = layout;
        }
    }

    /// Give the file the buffer its bytes arrived in.
    pub fn attach(
        &mut self,
        state: Entity<EditorState>,
        baseline: String,
        truncated: bool,
        version: Option<FileVersion>,
        change: Subscription,
    ) {
        self.body = FileBody::Text {
            state,
            baseline,
            truncated,
            version,
        };
        self.save = SaveState::Idle;
        self.dirty = false;
        self._change = Some(change);
    }

    /// Bytes the editor will not show. There is nothing to edit and nothing to save.
    pub fn set_binary(&mut self) {
        self.body = FileBody::Binary;
        self.dirty = false;
        self._change = None;
    }

    /// Give the tab bytes to draw rather than a buffer to edit.
    ///
    /// This is what a file whose viewer is a decoder gets instead of [`OpenFile::set_binary`]: an
    /// image is not text and is not nothing, and the difference is which of the two it is handed.
    pub fn set_bytes(&mut self, bytes: Vec<u8>) {
        self.body = FileBody::Bytes(bytes);
        self.save = SaveState::Idle;
        self.dirty = false;
        self._change = None;
    }

    /// Whether the tab's bytes go to a viewer rather than into a buffer. A read still has to
    /// happen; what changes is what is done with the answer.
    pub fn draws_bytes(&self) -> bool {
        matches!(self.viewer, ViewerKind::Image)
    }

    /// The read failed, and the tab says why rather than sitting empty.
    pub fn set_failed(&mut self, reason: String) {
        self.body = FileBody::Failed(reason);
        self.dirty = false;
        self._change = None;
    }

    /// Whether the tab is still waiting for its bytes. A tab that has them is never overwritten by
    /// a late arrival, because that would discard whatever has been typed into it.
    pub fn is_loading(&self) -> bool {
        matches!(self.body, FileBody::Loading)
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// Whether a save would be honest. A truncated read is a prefix, and writing a prefix back
    /// would shorten the file. A buffer with no version has nothing to hand back either, and a
    /// write naming no version is refused anyway — under a real host reply the two conditions
    /// coincide, but a guest file is the first case that is un-truncated and version-less both.
    pub fn savable(&self) -> bool {
        matches!(
            self.body,
            FileBody::Text {
                truncated: false,
                version: Some(_),
                ..
            }
        )
    }

    /// The buffer, for the one module that draws it.
    pub fn buffer(&self) -> Option<&Entity<EditorState>> {
        match &self.body {
            FileBody::Text { state, .. } => Some(state),
            _ => None,
        }
    }

    /// The version a save has to name.
    pub fn version(&self) -> Option<FileVersion> {
        match &self.body {
            FileBody::Text { version, .. } => *version,
            _ => None,
        }
    }

    /// Recompute dirty from what the buffer now holds.
    ///
    /// Driven by the buffer's own change event, and given the text rather than reading it, so this
    /// module needs no context. Answering `true` means the first edit promoted a temporary preview
    /// to a permanent tab, so the pane can forget the preview its `temporary_key` named.
    pub fn refresh_dirty(&mut self, text: &str) -> bool {
        // The first edit promotes a temporary preview to a permanent tab. A preview that is still
        // showing its file unchanged stays replaceable.
        let mut promoted = false;
        if self.temporary
            && let Some(base) = self.baseline()
            && text != base
        {
            self.temporary = false;
            promoted = true;
        }
        if let FileBody::Text { baseline, .. } = &self.body {
            self.dirty = text != baseline;
        }
        // An edit is the answer to a failed save: the user has moved on, and a stale error beside
        // a changed buffer says nothing true.
        if matches!(self.save, SaveState::Failed(_)) {
            self.save = SaveState::Idle;
        }
        promoted
    }

    /// The last text the host actually sent, when there is one.
    pub fn baseline(&self) -> Option<&str> {
        match &self.body {
            FileBody::Text { baseline, .. } => Some(baseline),
            _ => None,
        }
    }

    /// The buffer is on its way to the host.
    pub fn mark_saving(&mut self, text: String) {
        self.save = SaveState::Saving(text);
    }

    /// The host wrote it.
    ///
    /// The baseline becomes the text that was actually written, which is why the in-flight copy was
    /// kept — and dirty is recomputed against what the buffer holds *now*, because anything typed
    /// while the write was in flight is still unsaved and has to keep saying so.
    pub fn saved(&mut self, written: FileVersion, current: &str) {
        let text = match std::mem::replace(&mut self.save, SaveState::Idle) {
            SaveState::Saving(text) => text,
            _ => return,
        };
        if let FileBody::Text {
            baseline, version, ..
        } = &mut self.body
        {
            *version = Some(written);
            *baseline = text;
            self.dirty = current != baseline;
        }
    }

    /// Put the tab back to waiting, so a fresh read fills it — the file changed underneath it.
    ///
    /// Only ever called on a tab with nothing unsaved: a dirty buffer is never dropped for what
    /// is on disk.
    pub fn reload(&mut self) {
        self.body = FileBody::Loading;
        self.save = SaveState::Idle;
        self.dirty = false;
        self._change = None;
    }

    /// Where the cursor and scroll were, for [`OpenFile::reload`] to hand back to whatever buffer
    /// replaces this one. The caller reads these off the buffer this tab is about to lose.
    pub fn set_restore(&mut self, selection: Range<usize>, scroll: Point<Pixels>) {
        self.restore = Some((selection, scroll));
    }

    /// The position a fresh buffer should be put at, once — reading it twice would put a second
    /// tab's fresh buffer at the first tab's spot.
    pub fn take_restore(&mut self) -> Option<(Range<usize>, Point<Pixels>)> {
        self.restore.take()
    }

    /// The host refused. The buffer is untouched and the file is still dirty.
    pub fn save_failed(&mut self, reason: String) {
        self.save = SaveState::Failed(reason);
    }

    /// Toggle whether the YAML frontmatter disclosure is open.
    pub fn toggle_frontmatter(&mut self) {
        self.frontmatter_open = !self.frontmatter_open;
    }
}

pub struct EditorPaneState {
    pub open: Vec<OpenFile>,
    pub active: usize,
    /// The key of the current temporary preview tab, if any. Only one preview exists at a time.
    pub temporary_key: Option<String>,
}

impl EditorPaneState {
    /// No files open, which is every project until one is clicked or restored.
    pub fn empty() -> Self {
        Self {
            open: Vec::new(),
            active: 0,
            temporary_key: None,
        }
    }

    pub fn active_file(&self) -> Option<&OpenFile> {
        self.open.get(self.active)
    }

    pub fn active_file_mut(&mut self) -> Option<&mut OpenFile> {
        self.open.get_mut(self.active)
    }

    /// Where the tab looking at the file itself is.
    pub fn index_of(&self, path: &str) -> Option<usize> {
        self.index_of_key(&tab_key(path, Subject::File))
    }

    /// Where the tab with this key is, whatever it is looking at.
    pub fn index_of_key(&self, key: &str) -> Option<usize> {
        self.open.iter().position(|file| file.key() == key)
    }

    /// The tab looking at the file itself. Bytes arrive for a path, and a diff of the same path is
    /// a different tab that must not be filled with them.
    pub fn find_mut(&mut self, path: &str) -> Option<&mut OpenFile> {
        self.find_key_mut(&tab_key(path, Subject::File))
    }

    pub fn find_key_mut(&mut self, key: &str) -> Option<&mut OpenFile> {
        self.open.iter_mut().find(|file| file.key() == key)
    }

    /// Put a tab in front of the user before its bytes exist, answering the index it took. A path
    /// already open answers where it is instead, so a second click cannot open it twice.
    pub fn open_pending(&mut self, path: &str, markdown_open: ViewLayout) -> usize {
        match self.index_of_key(&tab_key(path, Subject::File)) {
            Some(at) => at,
            None => {
                self.open
                    .push(OpenFile::opening(path, Subject::File, markdown_open));
                self.open.len() - 1
            }
        }
    }

    /// The same, for a tab looking at something the host will make from the file.
    pub fn open_pending_on(&mut self, path: &str, subject: Subject) -> usize {
        match self.index_of_key(&tab_key(path, subject)) {
            Some(at) => at,
            None => {
                self.open.push(OpenFile::pending_on(path, subject));
                self.open.len() - 1
            }
        }
    }

    /// Open a temporary preview on a path, keeping at most one preview tab.
    ///
    /// A path already open answers where it is, keeping it permanent. The preview already open is
    /// replaced by this one: the returned `closed` names the tab whose panel the caller has to
    /// take away, `None` when nothing was displaced.
    pub fn open_temporary(
        &mut self,
        path: &str,
        markdown_open: ViewLayout,
    ) -> (usize, Option<String>) {
        let key = tab_key(path, Subject::File);
        if let Some(at) = self.index_of_key(&key) {
            return (at, None);
        }
        let closed = self.temporary_key.clone();
        if let Some(closed) = &closed
            && let Some(at) = self.index_of_key(closed)
        {
            self.open.remove(at);
            if self.active >= self.open.len() {
                self.active = self.open.len().saturating_sub(1);
            } else if at < self.active {
                self.active -= 1;
            }
        }
        self.open.push(OpenFile::temporary(path, markdown_open));
        self.temporary_key = Some(key);
        (self.open.len() - 1, closed)
    }

    /// Close a tab, keeping the active index pointing at something that still exists.
    pub fn close(&mut self, index: usize) {
        if index >= self.open.len() {
            return;
        }
        if self.open[index].key() == self.temporary_key.clone().unwrap_or_default() {
            self.temporary_key = None;
        }
        self.open.remove(index);
        if self.active >= self.open.len() {
            self.active = self.open.len().saturating_sub(1);
        } else if index < self.active {
            self.active -= 1;
        }
    }

    /// Make the preview for a path permanent, if it is one. Also forget it as the preview, so
    /// opening another file as a preview can no longer displace a tab the user has since kept.
    pub fn promote(&mut self, path: &str) -> bool {
        self.promote_key(&tab_key(path, Subject::File))
    }

    /// [`Self::promote`] by tab key.
    pub fn promote_key(&mut self, key: &str) -> bool {
        let promoted = self.find_key_mut(key).is_some_and(|file| {
            if file.temporary {
                file.temporary = false;
                true
            } else {
                false
            }
        });
        if promoted && self.temporary_key.as_deref() == Some(key) {
            self.temporary_key = None;
        }
        promoted
    }

    /// The tab order, as the project remembers it.
    pub fn paths(&self) -> Vec<String> {
        self.open.iter().map(|file| file.path.clone()).collect()
    }

    /// Which tab was in front. A path rather than an index, because a file that fails to open must
    /// not shift what "active" meant.
    pub fn active_path(&self) -> Option<String> {
        self.active_file().map(|file| file.path.clone())
    }
}

/// The last segment of a project-relative path, which is what a tab is called.
fn leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The lower-cased extension, or an empty string when there is none.
fn extension(path: &str) -> String {
    let name = leaf(path);
    match name.rsplit_once('.') {
        // A dotfile with no extension — `.gitignore` — is a name, not an extension.
        Some((stem, ext)) if !stem.is_empty() => ext.to_lowercase(),
        _ => String::new(),
    }
}
