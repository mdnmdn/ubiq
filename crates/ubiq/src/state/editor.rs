//! The files open in the centre pane.
//!
//! Each open file keeps its own buffer, so switching tabs does not lose what the previous one held.

use super::explorer::GitStatus;

/// The languages the scaffold opens. The mapping onto the highlighter's own enum lives in
/// `ui/editor.rs` — this module stays free of the component library.
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
}

pub struct OpenFile {
    pub name: String,
    pub path: String,
    pub language: FileLanguage,
    pub git: GitStatus,
    pub dirty: bool,
    pub source: String,
}

pub struct EditorPaneState {
    pub open: Vec<OpenFile>,
    pub active: usize,
}

impl EditorPaneState {
    pub fn new(open: Vec<OpenFile>) -> Self {
        Self { open, active: 0 }
    }

    pub fn active_file(&self) -> Option<&OpenFile> {
        self.open.get(self.active)
    }

    pub fn active_file_mut(&mut self) -> Option<&mut OpenFile> {
        self.open.get_mut(self.active)
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
    }
}
