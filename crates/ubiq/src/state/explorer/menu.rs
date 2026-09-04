use super::tree::*;
use super::*;

impl ExplorerState {
    /// Raise the menu at the pointer, remembering enough of the row to draw it after the tree
    /// has moved on.
    pub fn open_menu(&mut self, path: Option<&str>, x: f32, y: f32) {
        let (is_dir, readable, expanded) = match path {
            Some(path) => match node_of(&self.root, path) {
                Some(node) => {
                    let (expanded, _, _) = dir_flags(node);
                    (node.is_dir(), node.readable, expanded)
                }
                None => (false, false, false),
            },
            None => (false, true, false),
        };
        if let Some(path) = path {
            self.cursor = Some(path.to_string());
        }
        self.menu = Some(ExplorerMenu {
            path: path.map(str::to_string),
            is_dir,
            readable,
            expanded,
            x,
            y,
        });
        self.menu_held = true;
    }

    pub fn close_menu(&mut self) {
        if self.menu_held {
            self.menu_held = false;
            return;
        }
        self.menu = None;
    }
}

/// What a right-click offers for this row — or for the empty panel, when `path` is absent.
pub fn menu_entries(
    path: Option<&str>,
    is_dir: bool,
    readable: bool,
    expanded: bool,
) -> Vec<ExplorerEntry> {
    let entry = |action| ExplorerEntry { action, expanded };
    match path {
        None => vec![
            entry(ExplorerAction::NewFile),
            entry(ExplorerAction::NewFolder),
            entry(ExplorerAction::CollapseAll),
        ],
        Some(_) if is_dir => {
            let mut items = vec![entry(ExplorerAction::Toggle)];
            if readable {
                items.extend([
                    entry(ExplorerAction::NewFile),
                    entry(ExplorerAction::NewFolder),
                ]);
            }
            items.push(entry(ExplorerAction::CopyPath));
            items.push(entry(ExplorerAction::CopyFullPath));
            items.push(entry(ExplorerAction::OpenInSystem));
            items.push(entry(ExplorerAction::OpenInWeb));
            if readable {
                items.push(entry(ExplorerAction::Refresh));
            }
            if readable {
                items.extend([entry(ExplorerAction::Rename), entry(ExplorerAction::Delete)]);
            }
            items
        }
        Some(_) => {
            let mut items = Vec::new();
            if readable {
                items.extend([entry(ExplorerAction::Open), entry(ExplorerAction::OpenDiff)]);
            }
            items.push(entry(ExplorerAction::CopyPath));
            items.push(entry(ExplorerAction::CopyFullPath));
            items.push(entry(ExplorerAction::OpenInSystem));
            items.push(entry(ExplorerAction::OpenInWeb));
            if readable {
                items.extend([entry(ExplorerAction::Rename), entry(ExplorerAction::Delete)]);
            }
            items
        }
    }
}
