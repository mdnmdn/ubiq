//! The search contract: the query, its scope, and what comes back.
//!
//! `Query` is shared between project search and find-in-file, so the same ticks mean the same
//! matches in a buffer and on disk. The search family's message variants live in
//! [`crate::messages`]; this module owns the types that travel inside them.

use serde::{Deserialize, Serialize};

/// A search query with its four options. Shared between project search and find-in-file.
///
/// `regex` off means the text is a literal. `whole_word` puts a word boundary on either side of
/// whatever the other two produced. `case_sensitive` off folds case. There is no fifth.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Query {
    pub text: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// What to search.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    /// The project's files.
    Files,
    /// Everything the host can search — in v1, the files.
    Project,
}

/// What was actually searched, reported back on [`crate::messages::Message::SearchFinished`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    File,
    Task,
    Chat,
    Kb,
}

/// A batch of results from one source.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Batch {
    Files(Vec<FileHit>),
    /// Later.
    Tasks(Vec<TaskHit>),
}

/// One file's hits.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileHit {
    pub rel_path: String,
    pub lines: Vec<LineHit>,
    /// Whether the file had more hits than the ceiling allowed.
    pub truncated: bool,
}

/// A later source's hits — placeholder for when tasks, chats and the knowledge base arrive.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskHit {
    pub task_id: crate::ids::TaskId,
    pub field: String,
    pub text: String,
}

/// One line's match, with highlight ranges.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LineHit {
    /// 1-based line number.
    pub line: u32,
    /// The full line text.
    pub text: String,
    /// Byte offset ranges within `text` that matched, half-open.
    pub ranges: Vec<(u32, u32)>,
}

/// Search-level failures.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SearchError {
    /// The project's root is gone or unreadable.
    Root,
    /// The query could not be compiled.
    BadQuery(String),
    /// The file walk failed.
    Walk(String),
}
