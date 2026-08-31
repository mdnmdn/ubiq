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

use gpui::{Entity, Subscription};
use gpui_component::input::EditorState;
use ubiq_proto::files::FileVersion;

/// The languages the editor highlights. Anything else opens as plain text, which is the general
/// case rather than a fallback.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileLanguage {
    Tsx,
    TypeScript,
    Json,
    Rust,
    Markdown,
    Plain,
}

impl FileLanguage {
    /// What the status bar calls it.
    pub fn label(self) -> &'static str {
        match self {
            FileLanguage::Tsx | FileLanguage::TypeScript => "TypeScript",
            FileLanguage::Json => "JSON",
            FileLanguage::Rust => "Rust",
            FileLanguage::Markdown => "Markdown",
            FileLanguage::Plain => "Plain Text",
        }
    }

    /// The language a path's extension names.
    ///
    /// Only extensions the highlighter actually has a grammar for are named; everything else is
    /// plain text rather than highlighted as something it is not.
    pub fn of(path: &str) -> Self {
        match extension(path).as_str() {
            "tsx" | "jsx" => FileLanguage::Tsx,
            "ts" | "mts" | "cts" => FileLanguage::TypeScript,
            "json" | "jsonc" => FileLanguage::Json,
            "rs" => FileLanguage::Rust,
            "md" | "markdown" => FileLanguage::Markdown,
            _ => FileLanguage::Plain,
        }
    }
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
    pub language: FileLanguage,
    pub body: FileBody,
    pub save: SaveState,
    /// Cached rather than compared every frame: a per-frame comparison is the file's length times
    /// the tabs open times the frame rate.
    dirty: bool,
    /// The buffer's change event, which is what keeps `dirty` current. Held here because it must
    /// live exactly as long as the file does.
    _change: Option<Subscription>,
}

impl OpenFile {
    /// A tab with no bytes yet: what a click on the explorer produces before the host has answered.
    pub fn pending(path: &str) -> Self {
        Self {
            name: leaf(path).to_string(),
            path: path.to_string(),
            language: FileLanguage::of(path),
            body: FileBody::Loading,
            save: SaveState::Idle,
            dirty: false,
            _change: None,
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
    /// would shorten the file.
    pub fn savable(&self) -> bool {
        matches!(
            self.body,
            FileBody::Text {
                truncated: false,
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
    /// module needs no context.
    pub fn refresh_dirty(&mut self, text: &str) {
        if let FileBody::Text { baseline, .. } = &self.body {
            self.dirty = text != baseline;
        }
        // An edit is the answer to a failed save: the user has moved on, and a stale error beside
        // a changed buffer says nothing true.
        if matches!(self.save, SaveState::Failed(_)) {
            self.save = SaveState::Idle;
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

    /// The host refused. The buffer is untouched and the file is still dirty.
    pub fn save_failed(&mut self, reason: String) {
        self.save = SaveState::Failed(reason);
    }
}

pub struct EditorPaneState {
    pub open: Vec<OpenFile>,
    pub active: usize,
    /// A tab whose close is waiting on an answer, because its buffer holds unsaved changes. The
    /// close takes a second, explicit click rather than losing an edit silently.
    pub pending_tab_close: Option<String>,
}

impl EditorPaneState {
    /// No files open, which is every project until one is clicked or restored.
    pub fn empty() -> Self {
        Self {
            open: Vec::new(),
            active: 0,
            pending_tab_close: None,
        }
    }

    pub fn active_file(&self) -> Option<&OpenFile> {
        self.open.get(self.active)
    }

    pub fn active_file_mut(&mut self) -> Option<&mut OpenFile> {
        self.open.get_mut(self.active)
    }

    pub fn index_of(&self, path: &str) -> Option<usize> {
        self.open.iter().position(|file| file.path == path)
    }

    pub fn find_mut(&mut self, path: &str) -> Option<&mut OpenFile> {
        self.open.iter_mut().find(|file| file.path == path)
    }

    /// Put a tab in front of the user before its bytes exist, answering the index it took. A path
    /// already open answers where it is instead, so a second click cannot open it twice.
    pub fn open_pending(&mut self, path: &str) -> usize {
        match self.index_of(path) {
            Some(at) => at,
            None => {
                self.open.push(OpenFile::pending(path));
                self.open.len() - 1
            }
        }
    }

    /// Close a tab, keeping the active index pointing at something that still exists.
    pub fn close(&mut self, index: usize) {
        if index >= self.open.len() {
            return;
        }
        self.open.remove(index);
        if self.active >= self.open.len() {
            self.active = self.open.len().saturating_sub(1);
        } else if index < self.active {
            self.active -= 1;
        }
        self.pending_tab_close = None;
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
