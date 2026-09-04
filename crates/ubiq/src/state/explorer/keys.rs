use super::tree::*;
use super::*;

impl ExplorerState {
    /// Which row the keyboard is on, and where it is in what is drawn.
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// Where the cursor sits in the rows on screen, which is what a scroll has to be told.
    pub fn cursor_index(&self, filter: &str) -> Option<usize> {
        self.index_in(&self.visible_rows(filter))
    }

    /// The keyboard follows the mouse: an arrow after a click carries on from the row that was
    /// clicked, not from wherever the cursor was left.
    pub fn set_cursor(&mut self, path: &str) {
        self.cursor = Some(path.to_string());
        self.sync_hit_cursors();
    }

    /// What a click on a row means: a folder in the tree opens, a file is the thing to open, and
    /// a folder in the list is only where the cursor lands — there is no depth to walk into.
    pub fn click(&mut self, path: &str) -> ExplorerPressed {
        self.set_cursor(path);
        let (readable, is_dir) = match node_of(&self.root, path) {
            Some(node) => (node.readable, node.is_dir()),
            None => return ExplorerPressed::Ignored,
        };
        if !readable {
            return ExplorerPressed::Ignored;
        }
        if is_dir {
            if self.view == ExplorerView::Tree {
                return self.toggle_result(path);
            }
            return ExplorerPressed::Moved;
        }
        ExplorerPressed::Open {
            path: path.to_string(),
        }
    }

    fn toggle_result(&mut self, path: &str) -> ExplorerPressed {
        match self.toggle(path) {
            Toggle::Listing => ExplorerPressed::Listing {
                path: path.to_string(),
            },
            Toggle::Done => ExplorerPressed::Moved,
            Toggle::Missing => ExplorerPressed::Ignored,
        }
    }

    // ── the keyboard ────────────────────────────────────────────────

    /// What a key means here, and what is left for whoever else wants it.
    ///
    /// **Every rule is in this one function**, so the explorer behaves the same however the key
    /// arrived — and so `tests/explorer.rs` can press keys without a window.
    pub fn press(&mut self, key: ExplorerKey, filter: &str) -> ExplorerPressed {
        let pressed = match key {
            ExplorerKey::Dismiss => self.dismiss(filter),
            ExplorerKey::Up => self.step(-1, filter),
            ExplorerKey::Down => self.step(1, filter),
            ExplorerKey::Left => self.step_out(filter),
            ExplorerKey::Right => self.step_in(filter),
            ExplorerKey::Enter => self.enter(filter),
            ExplorerKey::ShiftEnter => self.enter(filter),
        };
        self.sync_hit_cursors();
        pressed
    }

    fn index_in(&self, rows: &[Row]) -> Option<usize> {
        let cursor = self.cursor.as_deref()?;
        rows.iter().position(|row| row.path == cursor)
    }

    /// One row up or down, stopping at the ends rather than wrapping — a list that wraps loses the
    /// user the moment they hold the key down.
    fn step(&mut self, delta: isize, filter: &str) -> ExplorerPressed {
        let rows = self.visible_rows(filter);
        if rows.is_empty() {
            return ExplorerPressed::Ignored;
        }
        let next = match self.index_in(&rows) {
            Some(at) => (at as isize + delta).clamp(0, rows.len() as isize - 1) as usize,
            None if delta > 0 => 0,
            None => rows.len() - 1,
        };
        self.cursor = Some(rows[next].path.clone());
        ExplorerPressed::Moved
    }

    /// Open the folder the cursor is on, or — where it is already open — step into it.
    fn step_in(&mut self, filter: &str) -> ExplorerPressed {
        let rows = self.visible_rows(filter);
        let Some(at) = self.index_in(&rows) else {
            return ExplorerPressed::Ignored;
        };
        let row = rows[at].clone();
        if self.view != ExplorerView::Tree || !row.is_dir {
            return ExplorerPressed::Ignored;
        }

        if self.needs_listing(&row.path) {
            return self.toggle_result(&row.path);
        }

        if !self.is_expanded(&row.path) && filter.trim().is_empty() {
            return self.toggle_result(&row.path);
        }

        match rows.get(at + 1).filter(|next| next.depth > row.depth) {
            Some(child) => {
                self.cursor = Some(child.path.clone());
                ExplorerPressed::Moved
            }
            None => ExplorerPressed::Ignored,
        }
    }

    /// Shut the folder the cursor is on, or step out to the folder holding it.
    fn step_out(&mut self, filter: &str) -> ExplorerPressed {
        let rows = self.visible_rows(filter);
        let Some(at) = self.index_in(&rows) else {
            return ExplorerPressed::Ignored;
        };
        let row = rows[at].clone();
        if self.view != ExplorerView::Tree {
            return ExplorerPressed::Ignored;
        }

        // While a filter is typed every folder is drawn open, so shutting one would change nothing
        // on screen. Stepping out still means something, and that is what it does.
        if row.is_dir && self.is_expanded(&row.path) && filter.trim().is_empty() {
            return self.toggle_result(&row.path);
        }

        let depth = row.depth;
        match rows[..at].iter().rposition(|above| above.depth < depth) {
            Some(parent) => {
                self.cursor = Some(rows[parent].path.clone());
                ExplorerPressed::Moved
            }
            None => ExplorerPressed::Ignored,
        }
    }

    fn enter(&mut self, filter: &str) -> ExplorerPressed {
        let rows = self.visible_rows(filter);
        let Some(at) = self.index_in(&rows) else {
            return ExplorerPressed::Ignored;
        };
        let row = rows[at].clone();
        if !row.readable {
            return ExplorerPressed::Ignored;
        }
        if row.is_dir {
            if self.view == ExplorerView::Tree {
                return self.toggle_result(&row.path);
            }
            return ExplorerPressed::Moved;
        }
        ExplorerPressed::Open { path: row.path }
    }

    fn dismiss(&mut self, filter: &str) -> ExplorerPressed {
        if self.menu.is_some() {
            self.menu = None;
            self.menu_held = false;
            return ExplorerPressed::Dismissed;
        }
        if !filter.trim().is_empty() {
            return ExplorerPressed::ClearFilter;
        }
        ExplorerPressed::Ignored
    }
}
