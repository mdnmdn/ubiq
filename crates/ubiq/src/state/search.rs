//! The search panel's state: the query, its options, and the results so far.

use gpui::Entity;
use gpui_component::input::InputState;
use ubiq_proto::ids::{ProjectId, SearchId};
use ubiq_proto::search::{LineHit, SearchError};

/// A search panel's state, owned by [`crate::app::AppState`].
pub struct SearchState {
    /// The query input field.
    pub query: Entity<InputState>,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    /// The in-flight search, if any.
    pub active: Option<ActiveSearch>,
    /// Results grouped by file, in arrival order.
    pub results: Vec<FileResult>,
    /// How many files the walker has seen.
    pub files_seen: usize,
    /// Total hits across all files.
    pub total_hits: usize,
    /// Whether any ceiling was hit.
    pub truncated: bool,
    /// Whether the search is done.
    pub finished: bool,
    /// The error that stopped the search, if any.
    pub error: Option<SearchError>,
}

/// The search currently in flight.
pub struct ActiveSearch {
    pub search_id: SearchId,
    pub project_id: ProjectId,
}

/// One file's results, accumulated from batches.
pub struct FileResult {
    pub rel_path: String,
    pub hits: Vec<LineHit>,
    pub truncated: bool,
}

impl SearchState {
    pub fn new(query: Entity<InputState>) -> Self {
        Self {
            query,
            case_sensitive: false,
            whole_word: false,
            regex: false,
            active: None,
            results: Vec::new(),
            files_seen: 0,
            total_hits: 0,
            truncated: false,
            finished: false,
            error: None,
        }
    }

    /// Clear results and reset for a new search.
    pub fn reset(&mut self) {
        self.active = None;
        self.results.clear();
        self.files_seen = 0;
        self.total_hits = 0;
        self.truncated = false;
        self.finished = false;
        self.error = None;
    }
}
