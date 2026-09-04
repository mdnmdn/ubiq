use super::*;

impl ExplorerState {
    pub(super) fn hits_ref(&self, filter: &str) -> Option<&[Row]> {
        let hits = self.filter_hits.as_ref()?;
        if hits.view != self.view {
            return None;
        }
        if hits.needle != filter.trim().to_lowercase() {
            return None;
        }
        Some(&hits.rows)
    }

    pub(super) fn hits_for(&self, filter: &str) -> Option<Vec<Row>> {
        self.hits_ref(filter).map(|rows| rows.to_vec())
    }

    /// A snapshot the background thread can walk. Cloning the `Arc` is the point: the frame must
    /// not copy the tree to start a search.
    pub fn filter_snap(&self) -> FilterSnap {
        FilterSnap {
            root: Arc::clone(&self.root),
            root_name: self.root_name.clone(),
            root_expanded: self.root_expanded,
            view: self.view,
            cursor: self.cursor.clone(),
            selected: self.selected.clone(),
        }
    }

    /// Start a background filter walk. Answers the job id the result has to carry back.
    pub fn begin_filter(&mut self) -> u64 {
        self.filter_job = self.filter_job.wrapping_add(1);
        self.filter_job
    }

    /// Land a background walk, answering whether it was still the one that was asked for.
    pub fn apply_hits(
        &mut self,
        job: u64,
        filter: String,
        view: ExplorerView,
        rows: Vec<Row>,
    ) -> bool {
        if job != self.filter_job || self.view != view {
            return false;
        }
        self.filter_hits = Some(FilterHits {
            needle: filter.trim().to_lowercase(),
            view,
            rows,
        });
        self.reanchor_hits();
        self.sync_hit_cursors();
        true
    }

    /// Drop hits and cancel any walk still in flight. Clearing the field is immediate.
    pub fn clear_filter(&mut self) {
        self.filter_hits = None;
        self.filter_job = self.filter_job.wrapping_add(1);
    }

    fn reanchor_hits(&mut self) {
        let Some(hits) = &self.filter_hits else {
            return;
        };
        let held = self
            .cursor
            .as_deref()
            .is_some_and(|path| hits.rows.iter().any(|row| row.path == path));
        if !held {
            self.cursor = Self::first_cursor(&hits.rows);
        }
    }

    pub(super) fn sync_hit_cursors(&mut self) {
        let cursor = self.cursor.clone();
        if let Some(hits) = &mut self.filter_hits {
            for row in &mut hits.rows {
                row.on_cursor = cursor.as_deref() == Some(row.path.as_str());
            }
        }
    }
}
