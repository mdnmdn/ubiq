use super::tree::*;
use super::*;

impl ExplorerState {
    /// Raise the menu at the pointer, remembering enough of the row to draw it after the tree
    /// has moved on.
    pub fn open_menu(&mut self, path: Option<&str>, x: f32, y: f32) {
        let (is_dir, readable, expanded) = match path {
            // The project's own row: a folder, and the folder every create with no row in mind
            // already lands in. There is no node for the empty path, and reading one would give
            // it an unreadable file's menu.
            Some("") => (true, true, self.root_expanded),
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
        self.menu_epoch = self.menu_epoch.wrapping_add(1);
        self.menu = Some(ExplorerMenu {
            epoch: self.menu_epoch,
            path: path.map(str::to_string),
            is_dir,
            readable,
            expanded,
            can_paste: self.copied.is_some(),
            x,
            y,
        });
    }

    /// Take the menu away, if the menu that is up is still the one being dismissed.
    ///
    /// The epoch is what tells an outside click apart from the right-click that raised the next
    /// menu: that second click reaches the first menu's outside-click handler too, carrying the
    /// epoch of the menu that has already gone.
    pub fn close_menu(&mut self, epoch: u64) {
        if self.menu.as_ref().is_some_and(|menu| menu.epoch == epoch) {
            self.menu = None;
        }
    }
}

/// What a right-click offers for this row — or for the empty panel, when `path` is absent.
///
/// The order is what the pick reads: [`ExplorerMenu::entries`] is indexed by the row that was
/// clicked, so whether a row is offered and whether it is enabled are both decided here rather
/// than by whoever draws the list. A separator is an entry of its own for the same reason: the
/// drawing side enumerates every item it is given, so a line that occupied no slot here would
/// shift every action below it by one.
pub fn menu_entries(
    path: Option<&str>,
    is_dir: bool,
    readable: bool,
    can_paste: bool,
) -> Vec<ExplorerEntry> {
    let entry = |action| ExplorerEntry {
        action,
        enabled: true,
    };
    let paste = ExplorerEntry {
        action: ExplorerAction::Paste,
        enabled: can_paste,
    };
    // Whole groups, so one that has nothing in it takes its separator with it.
    let groups: Vec<Vec<ExplorerEntry>> = match path {
        None => vec![
            vec![
                entry(ExplorerAction::NewFile),
                entry(ExplorerAction::NewFolder),
            ],
            vec![paste],
            vec![entry(ExplorerAction::CollapseAll)],
        ],
        // A row the host will not follow keeps only the path group: there is nothing behind it to
        // open, list, copy or rename.
        Some(_) => vec![
            match readable && !is_dir {
                true => vec![entry(ExplorerAction::Open), entry(ExplorerAction::OpenDiff)],
                false => Vec::new(),
            },
            // New file and new folder land in the folder holding this row, which is what
            // `ExplorerState::target_dir` answers — so a file row offers them too rather than
            // making the user find its folder first.
            match readable {
                true => vec![
                    entry(ExplorerAction::NewFile),
                    entry(ExplorerAction::NewFolder),
                ],
                false => Vec::new(),
            },
            match readable {
                true => vec![
                    entry(ExplorerAction::Copy),
                    paste,
                    entry(ExplorerAction::Duplicate),
                ],
                false => Vec::new(),
            },
            vec![
                entry(ExplorerAction::CopyPath),
                entry(ExplorerAction::CopyFullPath),
                entry(ExplorerAction::OpenInSystem),
                entry(ExplorerAction::OpenInWeb),
            ],
            match readable && is_dir {
                true => vec![entry(ExplorerAction::Refresh)],
                false => Vec::new(),
            },
            match readable {
                true => vec![entry(ExplorerAction::Rename), entry(ExplorerAction::Delete)],
                false => Vec::new(),
            },
        ],
    };

    let mut items: Vec<ExplorerEntry> = Vec::new();
    for group in groups.into_iter().filter(|group| !group.is_empty()) {
        if !items.is_empty() {
            items.push(ExplorerEntry {
                action: ExplorerAction::Separator,
                enabled: false,
            });
        }
        items.extend(group);
    }
    items
}
