//! The file tree: how a listing lands in it, what survives a re-listing, and how a remembered set
//! of open folders is restored one level at a time.
//!
//! All of it without a frame. The tree is pure state precisely so that these rules can be asserted
//! rather than clicked through.

use ubiq::state::explorer::{
    ExplorerAction, ExplorerKey, ExplorerPressed, ExplorerState, ExplorerView, GitStatus, NodeKind,
    Toggle, menu_entries,
};
use ubiq_proto::files::{DirEntry, DirListing, EntryKind};
use ubiq_proto::git::{GitEntry, GitMark, GitPathChange, GitRollup};

fn rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

fn dir(parent: &str, name: &str) -> DirEntry {
    DirEntry {
        name: name.to_string(),
        rel_path: rel(parent, name),
        kind: EntryKind::Dir,
        size: None,
        symlink: false,
    }
}

fn file(parent: &str, name: &str) -> DirEntry {
    DirEntry {
        name: name.to_string(),
        rel_path: rel(parent, name),
        kind: EntryKind::File,
        size: Some(0),
        symlink: false,
    }
}

fn other(parent: &str, name: &str) -> DirEntry {
    DirEntry {
        name: name.to_string(),
        rel_path: rel(parent, name),
        kind: EntryKind::Other,
        size: None,
        symlink: true,
    }
}

fn listing(rel_path: &str, entries: Vec<DirEntry>) -> DirListing {
    DirListing {
        rel_path: rel_path.to_string(),
        entries,
        truncated: false,
    }
}

/// The rows under the project's own row, which is what every case about the tree's contents is
/// asking for. The root row is asserted on its own, in `the_project_is_the_tree_s_first_row`.
fn names(tree: &ExplorerState) -> Vec<String> {
    under_root(tree.rows(""))
}

fn under_root(rows: Vec<ubiq::state::Row>) -> Vec<String> {
    rows.into_iter()
        .filter(|row| !row.path.is_empty())
        .map(|row| row.path)
        .collect()
}

/// The host sorts directories first and names without case. The tree keeps that order exactly:
/// re-sorting here is what would let two windows disagree about one project.
#[test]
fn a_listing_keeps_the_order_the_host_sent() {
    let mut tree = ExplorerState::empty();
    assert!(!tree.is_listed());

    tree.merge(listing(
        "",
        vec![
            dir("", "_docs"),
            dir("", "crates"),
            file("", "justfile"),
            file("", "README.md"),
        ],
    ));

    assert!(tree.is_listed());
    assert_eq!(names(&tree), ["_docs", "crates", "justfile", "README.md"]);
}

/// A folder is asked about exactly once. The expand that opens it answers `Listing`; every flip
/// after that answers `Done`, because a request is already out or has already been answered.
#[test]
fn a_folder_is_asked_about_once() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing("", vec![dir("", "src")]));

    assert_eq!(tree.toggle("src"), Toggle::Listing);
    tree.set_loading("src", true);

    // Shut again and reopened while the answer is still in flight: nothing more to ask.
    assert_eq!(tree.toggle("src"), Toggle::Done);
    assert_eq!(tree.toggle("src"), Toggle::Done);

    tree.merge(listing("src", vec![file("src", "main.rs")]));
    assert_eq!(tree.toggle("src"), Toggle::Done);
    assert_eq!(tree.toggle("src"), Toggle::Done);

    assert_eq!(tree.toggle("nowhere"), Toggle::Missing);
}

/// Re-listing a folder is not rebuilding it. Everything the host has already said about what is
/// below it, and everything the user has opened, survives — which is what makes a restore, or one
/// day a filesystem watch, idempotent rather than destructive.
#[test]
fn a_re_listing_keeps_what_is_known_below_it() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing("", vec![dir("", "src")]));
    tree.toggle("src");
    tree.merge(listing(
        "src",
        vec![dir("src", "panels"), file("src", "sessions.ts")],
    ));
    tree.toggle("src/panels");
    tree.merge(listing(
        "src/panels",
        vec![file("src/panels", "Terminal.tsx")],
    ));

    let before = names(&tree);
    assert!(before.contains(&"src/panels/Terminal.tsx".to_string()));

    // The same root listing arriving again.
    tree.merge(listing("", vec![dir("", "src")]));

    assert_eq!(names(&tree), before);
    assert_eq!(tree.expanded(), ["src", "src/panels"]);
}

/// A name a listing does not carry has gone, and its subtree goes with it. Nothing is kept on the
/// chance it comes back.
#[test]
fn an_entry_the_listing_no_longer_carries_is_dropped_with_its_subtree() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing("", vec![dir("", "src")]));
    tree.toggle("src");
    tree.merge(listing(
        "src",
        vec![dir("src", "panels"), file("src", "sessions.ts")],
    ));
    tree.toggle("src/panels");
    tree.merge(listing(
        "src/panels",
        vec![file("src/panels", "Terminal.tsx")],
    ));

    tree.merge(listing("src", vec![file("src", "sessions.ts")]));

    assert_eq!(names(&tree), ["src", "src/sessions.ts"]);
    assert_eq!(tree.expanded(), ["src"]);
}

/// A name that arrives in a re-listing is new: shut, and unlisted, so opening it asks the host
/// rather than drawing an empty folder.
#[test]
fn a_new_entry_arrives_shut_and_unlisted() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing("", vec![dir("", "src")]));
    tree.toggle("src");
    tree.merge(listing("src", vec![file("src", "sessions.ts")]));

    tree.merge(listing(
        "src",
        vec![dir("src", "panels"), file("src", "sessions.ts")],
    ));

    let panels = tree
        .rows("")
        .into_iter()
        .find(|row| row.path == "src/panels")
        .expect("the new folder is drawn");
    assert!(panels.is_dir);
    assert!(!panels.expanded);
    assert_eq!(tree.toggle("src/panels"), Toggle::Listing);
}

/// A folder collapsed away while its listing was in flight has nowhere to put the answer, and
/// saying so is how the window knows not to stop a spinner that is no longer drawn.
#[test]
fn a_listing_for_a_folder_the_tree_does_not_hold_is_refused() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing("", vec![dir("", "src")]));

    assert!(!tree.merge(listing("gone", vec![file("gone", "x")])));
    assert!(!tree.merge(listing("src/deep/deeper", vec![])));
    // A file is not a folder, whatever the listing says.
    tree.merge(listing("src", vec![file("src", "main.rs")]));
    assert!(!tree.merge(listing("src/main.rs", vec![])));

    assert!(tree.merge(listing("src", vec![file("src", "main.rs")])));
}

/// A remembered folder cannot be opened before its parents have been listed, so the set is worked
/// through again after every answer and shrinks as it goes.
#[test]
fn a_remembered_folder_is_reopened_as_its_parents_arrive() {
    let mut tree = ExplorerState::empty();
    let mut wanted = vec![
        "crates/ubiq".to_string(),
        "crates".to_string(),
        "renamed".to_string(),
    ];

    tree.merge(listing("", vec![dir("", "crates")]));

    // The root's answer reaches `crates` and no further; `renamed` is not in a root that has now
    // been listed, so it is gone rather than pending.
    assert_eq!(tree.reopen(&mut wanted), ["crates"]);
    assert_eq!(wanted, ["crates/ubiq"]);

    // Asking again before the answer arrives asks for nothing: the folder is already marked as
    // loading.
    assert!(tree.reopen(&mut wanted).is_empty());
    assert_eq!(wanted, ["crates/ubiq"]);

    tree.merge(listing(
        "crates",
        vec![dir("crates", "ubiq"), dir("crates", "ubiq-host")],
    ));

    assert_eq!(tree.reopen(&mut wanted), ["crates/ubiq"]);
    assert!(wanted.is_empty());
    assert_eq!(tree.expanded(), ["crates", "crates/ubiq"]);
}

/// A remembered folder below one that has not answered yet waits, rather than being dropped as
/// though it had been deleted. The two are told apart by whether the parent has been listed.
#[test]
fn a_folder_below_an_unlisted_parent_waits_rather_than_going() {
    let mut tree = ExplorerState::empty();
    let mut wanted = vec!["crates/ubiq/src".to_string()];

    // Nothing has been listed at all: not even the root can say the path is wrong.
    assert!(tree.reopen(&mut wanted).is_empty());
    assert_eq!(wanted, ["crates/ubiq/src"]);

    tree.merge(listing("", vec![dir("", "crates")]));
    assert!(tree.reopen(&mut wanted).is_empty());
    assert_eq!(wanted, ["crates/ubiq/src"]);

    tree.merge(listing("crates", vec![file("crates", "ubiq")]));
    // `ubiq` turned out to be a file, so the remembered folder below it cannot exist.
    assert!(tree.reopen(&mut wanted).is_empty());
    assert!(wanted.is_empty());
}

/// Filtering finds rather than prunes: it reaches into shut folders and answers with the matches,
/// and only what the host has already named can match.
#[test]
fn a_filter_finds_inside_shut_folders() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing("", vec![dir("", "src"), file("", "justfile")]));
    tree.merge(listing(
        "src",
        vec![file("src", "sessions.ts"), file("src", "main.rs")],
    ));
    // `src` is shut, so an unfiltered tree shows only the folder itself.
    assert_eq!(names(&tree), ["src", "justfile"]);

    // The match is reached even though its folder is shut, and the folder is kept as the way in.
    assert_eq!(
        under_root(tree.rows("sessions")),
        ["src", "src/sessions.ts"]
    );

    // A folder that matches is drawn open, because its children are what the user is looking for.
    let rows = tree.rows("src");
    assert_eq!(
        under_root(rows.clone()),
        ["src", "src/sessions.ts", "src/main.rs"]
    );
    assert!(rows[1].expanded);

    // The project's row survives a filter that finds nothing, so the handle that collapses it
    // never disappears.
    let nothing = tree.rows("nothing-matches-this");
    assert_eq!(nothing.len(), 1);
    assert!(nothing[0].path.is_empty());
}

/// A filter that matches a folder the host has never listed answers with an empty folder, so the
/// window is told which folders to ask about. The walk's skip set is left alone.
#[test]
fn a_folder_the_filter_matched_is_asked_about_once() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing(
        "",
        vec![dir("", "sessions"), dir("", "node_modules")],
    ));

    let rows = tree.rows("session");
    let asking = tree.unlisted_hits(&rows);
    assert_eq!(asking, ["sessions"]);

    // Asked once: the second walk over the same rows asks for nothing.
    tree.begin_cache(&asking);
    assert!(tree.unlisted_hits(&rows).is_empty());

    // The skip set is never asked for, however well it matches.
    let skipped = tree.rows("node_mod");
    assert!(tree.unlisted_hits(&skipped).is_empty());
}

/// Collapsing is not forgetting. A folder shut and reopened draws immediately rather than asking
/// the host a second time for what it has already said.
#[test]
fn collapsing_keeps_what_the_host_has_said() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing("", vec![dir("", "src")]));
    tree.toggle("src");
    tree.merge(listing("src", vec![file("src", "main.rs")]));
    assert_eq!(tree.expanded(), ["src"]);

    tree.collapse_all();

    assert!(tree.expanded().is_empty());
    assert_eq!(tree.toggle("src"), Toggle::Done);
    assert_eq!(names(&tree), ["src", "src/main.rs"]);
}

/// Something the host will not follow — a symlink out of the project, a socket, a device — is
/// drawn rather than hidden, and marked, because a tree with rows missing is a tree that lies.
#[test]
fn an_entry_the_host_will_not_follow_is_drawn_and_marked() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing(
        "",
        vec![other("", "elsewhere"), file("", "justfile")],
    ));

    let rows = tree.rows("");
    assert_eq!(rows.len(), 3, "the project's own row and the two entries");
    assert!(!rows[1].readable);
    assert!(!rows[1].is_dir, "there is nothing to expand it into");
    assert!(rows[2].readable);
}

/// A listing the host's ceiling cut short says so on the folder, rather than drawing it as smaller
/// than it is.
#[test]
fn a_truncated_listing_is_marked_on_the_folder() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing("", vec![dir("", "huge")]));
    tree.toggle("huge");
    tree.merge(DirListing {
        rel_path: "huge".to_string(),
        entries: vec![file("huge", "one")],
        truncated: true,
    });

    let huge = tree
        .rows("")
        .into_iter()
        .find(|row| row.path == "huge")
        .expect("the folder is drawn");
    assert!(huge.truncated);
    assert!(!huge.loading, "the answer has arrived");
}

/// A folder waiting for its listing says so, so it does not read as an empty folder.
#[test]
fn a_folder_waiting_for_its_listing_says_so() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing("", vec![dir("", "src")]));
    tree.toggle("src");
    tree.set_loading("src", true);

    let row = tree
        .rows("")
        .into_iter()
        .find(|row| row.path == "src")
        .expect("the folder is drawn");
    assert!(row.loading);

    tree.merge(listing("src", vec![]));
    let row = tree
        .rows("")
        .into_iter()
        .find(|row| row.path == "src")
        .expect("the folder is drawn");
    assert!(!row.loading);
    // A folder that has been listed and is empty is not the same as one nobody has asked about.
    assert!(matches!(
        &tree.root[0].kind,
        NodeKind::Dir { listed: true, .. }
    ));
}

fn listed() -> ExplorerState {
    let mut tree = ExplorerState::empty();
    tree.merge(listing("", vec![dir("", "src"), file("", "justfile")]));
    tree.merge(listing(
        "src",
        vec![file("src", "sessions.ts"), file("src", "main.rs")],
    ));
    tree
}

/// The project is the tree's first row: one handle that collapses everything, and the folder a
/// create or a drop with no row in mind already lands in.
#[test]
fn the_project_is_the_tree_s_first_row() {
    let mut tree = listed();
    tree.root_name = "ubiq".to_string();
    tree.toggle("src");

    let rows = tree.rows("");
    assert_eq!(rows[0].path, "");
    assert_eq!(rows[0].name, "ubiq");
    assert_eq!(rows[0].depth, 0);
    assert!(rows[0].is_dir && rows[0].expanded && rows[0].readable);
    assert!(rows[0].git.is_none());
    // Everything the host named hangs under it, a level further in.
    assert_eq!(rows[1].path, "src");
    assert_eq!(rows[1].depth, 1);
    assert_eq!(rows[2].depth, 2);

    // Shut, it is the only row there is — and the tree below it is not forgotten.
    assert_eq!(tree.toggle(""), Toggle::Done);
    assert!(!tree.root_expanded);
    let shut = tree.rows("");
    assert_eq!(shut.len(), 1);
    assert!(!shut[0].expanded);

    assert_eq!(tree.toggle(""), Toggle::Done);
    assert_eq!(tree.rows("").len(), 5);

    // The row an action on it lands in is the project's top level.
    assert_eq!(tree.target_dir(""), "");
}

/// The list is flat by definition: there is no root row to hang anything under.
#[test]
fn the_flat_list_has_no_project_row() {
    let mut tree = listed();
    tree.root_name = "ubiq".to_string();
    tree.set_view(ExplorerView::List, "");
    assert!(tree.rows("").iter().all(|row| !row.path.is_empty()));
}

/// A right-click on the project's row offers a folder's menu. There is no node behind the empty
/// path, and reading one would give it the menu of an unreadable file.
#[test]
fn the_project_row_offers_a_folder_s_menu() {
    let mut tree = listed();
    tree.open_menu(Some(""), 0.0, 0.0);
    let menu = tree.menu.clone().expect("a menu is up");
    assert!(menu.is_dir && menu.readable && menu.expanded);
    assert_eq!(
        menu.entries().iter().map(|e| e.action).collect::<Vec<_>>(),
        menu_entries(Some("src"), true, true, false)
            .iter()
            .map(|e| e.action)
            .collect::<Vec<_>>()
    );
}

/// The keyboard reaches the project's row like any other: left shuts it, right opens it, and
/// Enter is the toggle rather than an attempt to open a file.
#[test]
fn the_keyboard_collapses_the_project_row() {
    let mut tree = listed();
    tree.set_cursor("");

    assert_eq!(tree.press(ExplorerKey::Left, ""), ExplorerPressed::Moved);
    assert_eq!(tree.rows("").len(), 1, "collapsed to the project alone");

    assert_eq!(tree.press(ExplorerKey::Right, ""), ExplorerPressed::Moved);
    assert!(tree.rows("").len() > 1);

    assert_eq!(tree.press(ExplorerKey::Enter, ""), ExplorerPressed::Moved);
    assert_eq!(tree.rows("").len(), 1);

    // A click on the row is the same toggle, and never an open.
    assert_eq!(tree.click(""), ExplorerPressed::Moved);
    assert!(tree.rows("").len() > 1);
}

/// The list is every match the host has already named, flat, whatever is open — and each row says
/// which folder it came from, because a flat list of names is ambiguous the moment two folders
/// agree on one.
#[test]
fn the_list_is_flat_and_says_which_folder_each_row_came_from() {
    let mut tree = listed();
    tree.set_view(ExplorerView::List, "");
    let rows = tree.rows("");

    assert!(rows.iter().all(|row| row.depth == 0), "the list indented");

    let deep = rows
        .iter()
        .find(|row| row.name == "sessions.ts")
        .expect("a file nothing was opened to reach");
    assert_eq!(deep.trailing, "src");

    let top = rows
        .iter()
        .find(|row| row.name == "justfile")
        .expect("the file at the top");
    assert_eq!(top.trailing, ".", "a file at the root says so");

    let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["justfile", "main.rs", "sessions.ts", "src"]);
}

/// Up and down walk the rows on screen and stop at the ends.
#[test]
fn the_arrows_walk_the_rows_and_stop_at_the_ends() {
    let mut tree = listed();
    tree.toggle("src");
    let rows = tree.rows("");

    assert_eq!(tree.press(ExplorerKey::Down, ""), ExplorerPressed::Moved);
    assert_eq!(tree.cursor(), Some(rows[0].path.as_str()));

    assert_eq!(tree.press(ExplorerKey::Down, ""), ExplorerPressed::Moved);
    assert_eq!(tree.cursor(), Some(rows[1].path.as_str()));

    tree.press(ExplorerKey::Up, "");
    assert_eq!(tree.cursor(), Some(rows[0].path.as_str()));
    tree.press(ExplorerKey::Up, "");
    assert_eq!(tree.cursor_index(""), Some(0));
}

/// Right opens the folder the cursor is on, then steps into it; left shuts it, then steps out.
#[test]
fn left_and_right_walk_the_tree_in_and_out() {
    let mut tree = listed();
    // The first row is the project itself, so the first folder is one step further down.
    tree.press(ExplorerKey::Down, "");
    tree.press(ExplorerKey::Down, "");
    assert_eq!(tree.cursor(), Some("src"));

    assert_eq!(tree.press(ExplorerKey::Right, ""), ExplorerPressed::Moved);
    assert!(names(&tree).contains(&"src/main.rs".to_string()), "shut");
    assert_eq!(tree.cursor(), Some("src"), "opening also moved");

    tree.press(ExplorerKey::Right, "");
    assert_eq!(tree.cursor(), Some("src/sessions.ts"));

    assert_eq!(tree.press(ExplorerKey::Right, ""), ExplorerPressed::Ignored);

    assert_eq!(tree.press(ExplorerKey::Left, ""), ExplorerPressed::Moved);
    assert_eq!(tree.cursor(), Some("src"));
    tree.press(ExplorerKey::Left, "");
    assert!(!names(&tree).contains(&"src/main.rs".to_string()), "open");
    assert_eq!(tree.cursor(), Some("src"), "shutting also moved");
}

/// The flat list has no depth to walk, so left and right mean nothing there — and saying so is
/// what gives the filter field its caret keys back.
#[test]
fn left_and_right_are_handed_back_in_the_flat_list() {
    let mut tree = listed();
    tree.set_view(ExplorerView::List, "");
    tree.press(ExplorerKey::Down, "");
    assert_eq!(tree.press(ExplorerKey::Left, ""), ExplorerPressed::Ignored);
    assert_eq!(tree.press(ExplorerKey::Right, ""), ExplorerPressed::Ignored);
    assert_eq!(tree.press(ExplorerKey::Down, ""), ExplorerPressed::Moved);
}

/// Enter on a file is opening it; enter on a folder in the tree toggles it.
#[test]
fn enter_opens_a_file_and_toggles_a_folder() {
    let mut tree = listed();
    tree.press(ExplorerKey::Down, "");
    tree.press(ExplorerKey::Down, "");
    assert_eq!(tree.cursor(), Some("src"));
    assert_eq!(tree.press(ExplorerKey::Enter, ""), ExplorerPressed::Moved);
    assert!(names(&tree).contains(&"src/main.rs".to_string()));

    tree.press(ExplorerKey::Right, "");
    assert_eq!(
        tree.press(ExplorerKey::Enter, ""),
        ExplorerPressed::Open {
            path: "src/sessions.ts".to_string()
        }
    );
}

/// A cursor left on a row that has gone is a cursor pointing at nothing, so filtering and
/// switching views both put it back on something that is there.
#[test]
fn the_cursor_is_put_back_on_a_row_that_is_still_drawn() {
    let mut tree = listed();
    tree.toggle("src");
    tree.set_cursor("src/main.rs");
    assert_eq!(tree.cursor(), Some("src/main.rs"));

    tree.reanchor("main");
    assert_eq!(tree.cursor(), Some("src/main.rs"));

    tree.reanchor("justfile");
    assert_eq!(tree.cursor(), Some("justfile"));
}

/// The right-click menu offers every gesture the row has, grouped, in the order the pick reads.
/// Paste is the one row that can be drawn and refused: with nothing copied there is nothing to
/// paste.
#[test]
fn a_right_click_offers_every_gesture_in_groups_and_disables_paste_with_no_clipboard() {
    use ExplorerAction as A;

    let file = menu_entries(Some("src/main.rs"), false, true, false);
    assert_eq!(
        file.iter().map(|e| e.action).collect::<Vec<_>>(),
        [
            A::Open,
            A::OpenDiff,
            A::Separator,
            A::NewFile,
            A::NewFolder,
            A::Separator,
            A::Copy,
            A::Paste,
            A::Duplicate,
            A::Separator,
            A::CopyPath,
            A::CopyFullPath,
            A::OpenInSystem,
            A::OpenInWeb,
            A::Separator,
            A::Rename,
            A::Delete,
        ]
    );
    assert!(
        file.iter()
            .filter(|e| !e.is_separator())
            .all(|e| e.enabled == (e.action != A::Paste)),
        "Paste is the one row an empty clipboard disables: {file:?}"
    );

    // A folder has nothing to open and something to refresh; there is no Expand row, because the
    // twisty and the row itself already say that.
    let folder = menu_entries(Some("src"), true, true, false);
    assert_eq!(
        folder.iter().map(|e| e.action).collect::<Vec<_>>(),
        [
            A::NewFile,
            A::NewFolder,
            A::Separator,
            A::Copy,
            A::Paste,
            A::Duplicate,
            A::Separator,
            A::CopyPath,
            A::CopyFullPath,
            A::OpenInSystem,
            A::OpenInWeb,
            A::Separator,
            A::Refresh,
            A::Separator,
            A::Rename,
            A::Delete,
        ]
    );

    let filled = menu_entries(Some("src"), true, true, true);
    assert!(
        filled.iter().any(|e| e.action == A::Paste && e.enabled),
        "with something copied, Paste is pickable"
    );

    // A row the host will not follow keeps the path group alone — and no separator, leading or
    // trailing, is left behind by the groups that dropped out.
    let unreadable = menu_entries(Some("elsewhere"), false, false, true);
    assert_eq!(
        unreadable.iter().map(|e| e.action).collect::<Vec<_>>(),
        [A::CopyPath, A::CopyFullPath, A::OpenInSystem, A::OpenInWeb]
    );

    let empty = menu_entries(None, false, true, false);
    assert_eq!(
        empty.iter().map(|e| e.action).collect::<Vec<_>>(),
        [
            A::NewFile,
            A::NewFolder,
            A::Separator,
            A::Paste,
            A::Separator,
            A::CollapseAll,
        ]
    );
    assert!(empty.last().expect("Collapse all is last").enabled);

    // No menu ever starts or ends with a line, and no two lines meet.
    for menu in [file, folder, filled, unreadable, empty] {
        assert!(!menu.first().expect("a menu has rows").is_separator());
        assert!(!menu.last().expect("a menu has rows").is_separator());
        assert!(
            !menu
                .windows(2)
                .any(|pair| pair[0].is_separator() && pair[1].is_separator()),
            "two separators in a row: {menu:?}"
        );
    }
}

/// The menu remembers whether there was anything to paste when it opened, so the pick — an index
/// into the list that was drawn — cannot land on a row that has since changed.
#[test]
fn the_menu_remembers_the_clipboard_it_opened_with() {
    let mut tree = listed();
    tree.open_menu(Some("src"), 0.0, 0.0);
    let before = tree.menu.as_ref().expect("a menu is up").entries();
    assert!(
        before
            .iter()
            .any(|e| e.action == ExplorerAction::Paste && !e.enabled)
    );

    tree.copied = Some("justfile".to_string());
    tree.open_menu(Some("src"), 0.0, 0.0);
    let after = tree.menu.as_ref().expect("a menu is up").entries();
    assert!(
        after
            .iter()
            .any(|e| e.action == ExplorerAction::Paste && e.enabled)
    );
}

/// The first click outside a menu closes it. The click that opened the menu never reaches the
/// dismiss — the handler is registered when the menu is painted — so nothing here is held back.
#[test]
fn one_click_outside_closes_the_menu() {
    let mut tree = listed();
    tree.open_menu(Some("src"), 0.0, 0.0);
    let epoch = tree.menu.as_ref().expect("a menu is up").epoch;

    tree.close_menu(epoch);
    assert!(tree.menu.is_none(), "the first outside click was swallowed");
}

/// Right-clicking a second row raises the new menu and fires the old one's outside-click for the
/// same event. That dismiss carries the epoch of the menu that has gone, and does nothing.
#[test]
fn a_dismiss_for_a_replaced_menu_leaves_the_new_one_alone() {
    let mut tree = listed();
    tree.open_menu(Some("src"), 0.0, 0.0);
    let stale = tree.menu.as_ref().expect("a menu is up").epoch;

    tree.open_menu(Some("justfile"), 5.0, 5.0);
    tree.close_menu(stale);

    let menu = tree.menu.as_ref().expect("the second menu is still up");
    assert_eq!(menu.path.as_deref(), Some("justfile"));
    assert_ne!(menu.epoch, stale);
}

/// Where an action on a row lands: a folder takes it itself, a file hands it to the folder holding
/// it, and a file at the top level hands it to the project's root.
#[test]
fn the_target_folder_is_the_row_or_the_one_holding_it() {
    let tree = listed();
    assert_eq!(tree.target_dir("src"), "src");
    assert_eq!(tree.target_dir("src/main.rs"), "src");
    assert_eq!(tree.target_dir("justfile"), "");
    // Nothing the tree holds: it is treated as a file, which is what a stale path is.
    assert_eq!(tree.target_dir("gone/away.rs"), "gone");
}

/// A copy is given a name the folder does not already hold, and the extension stays on the end.
#[test]
fn a_free_name_suffixes_until_the_folder_has_room() {
    let mut tree = listed();
    tree.merge(listing(
        "src",
        vec![
            file("src", "main.rs"),
            file("src", "main copy.rs"),
            file("src", "plain"),
        ],
    ));

    assert_eq!(tree.free_name("src", "other.rs"), "other.rs");
    assert_eq!(tree.free_name("src", "main.rs"), "main copy 2.rs");
    assert_eq!(tree.free_name("src", "plain"), "plain copy");
    // The root is the folder an empty parent names.
    assert_eq!(tree.free_name("", "justfile"), "justfile copy");
    // A folder nobody has listed can say nothing about collisions, so the name is taken as it is.
    assert_eq!(tree.free_name("vendor", "main.rs"), "main.rs");
}

/// Escape closes the menu first, then clears the filter, then is handed back.
#[test]
fn escape_dismisses_the_menu_then_the_filter() {
    let mut tree = listed();
    tree.open_menu(Some("justfile"), 10.0, 20.0);
    assert!(tree.menu.is_some());
    assert_eq!(
        tree.press(ExplorerKey::Dismiss, "just"),
        ExplorerPressed::Dismissed
    );
    assert!(tree.menu.is_none());

    assert_eq!(
        tree.press(ExplorerKey::Dismiss, "just"),
        ExplorerPressed::ClearFilter
    );
    assert_eq!(
        tree.press(ExplorerKey::Dismiss, ""),
        ExplorerPressed::Ignored
    );
}

/// The cache is filled in the background from project open: unopened folders are asked about,
/// skip-set folders are not, and a folder already asked about is not asked twice. A filter then
/// reads that cache rather than waiting on the host.
#[test]
fn the_cache_asks_for_unopened_folders_and_skips_the_walk_set() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing(
        "",
        vec![
            dir("", "src"),
            dir("", "node_modules"),
            file("", "justfile"),
        ],
    ));

    let asking = tree.unlisted_for_cache();
    assert_eq!(asking, ["src"]);
    assert!(
        !asking.iter().any(|p| p == "node_modules"),
        "the cache asked about the skip set"
    );

    tree.begin_cache(&asking);
    assert!(
        tree.unlisted_for_cache().is_empty(),
        "asked again while in flight"
    );

    tree.merge(listing(
        "src",
        vec![dir("src", "ui"), file("src", "main.rs")],
    ));
    assert_eq!(tree.unlisted_for_cache(), ["src/ui"]);

    // Once the files are named, a filter finds them even though nobody expanded the folder.
    let paths: Vec<String> = tree.rows("main").into_iter().map(|row| row.path).collect();
    assert!(paths.contains(&"src/main.rs".to_string()), "{paths:?}");
    assert!(
        !tree.rows("").iter().any(|row| row.path == "src/main.rs"),
        "an unfiltered tree showed what the cache listed"
    );
}

/// A background filter walk is keyed by a job, so a slow result cannot land on a query the user
/// has already left. The panel draws those hits rather than walking the tree again.
#[test]
fn a_filter_walk_is_keyed_and_drawn_from_hits() {
    let mut tree = listed();
    tree.merge(listing("src", vec![file("src", "main.rs")]));

    let job = tree.begin_filter();
    let rows = tree.rows("main");
    assert!(tree.apply_hits(job, "main".to_string(), tree.view, rows));
    let drawn: Vec<String> = tree
        .drawn_rows("main")
        .iter()
        .map(|row| row.path.clone())
        .collect();
    assert!(drawn.iter().any(|p| p == "src/main.rs"), "{drawn:?}");

    // A stale job does not replace what is drawn.
    let stale = job;
    let later = tree.begin_filter();
    assert_ne!(stale, later);
    assert!(!tree.apply_hits(stale, "main".to_string(), tree.view, Vec::new()));
    assert!(
        tree.drawn_rows("main")
            .iter()
            .any(|row| row.path == "src/main.rs")
    );

    tree.clear_filter();
    assert!(
        tree.drawn_rows("main").is_empty(),
        "cleared hits still drawn"
    );
}

fn git_entry(
    path: &str,
    worktree: Option<GitPathChange>,
    index: Option<GitPathChange>,
) -> GitEntry {
    GitEntry {
        rel_path: path.to_string(),
        index,
        worktree,
        conflicted: false,
        ignored: false,
    }
}

#[test]
fn a_working_tree_map_marks_matching_rows_and_leaves_the_rest_clean() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing("", vec![dir("", "src"), file("", "README.md")]));
    tree.toggle("src");
    tree.merge(listing("src", vec![file("src", "main.rs")]));

    assert!(tree.rows("").iter().all(|row| row.git.is_none()));

    tree.apply_git(
        1,
        &[git_entry(
            "src/main.rs",
            Some(GitPathChange::Modified),
            None,
        )],
        &[GitRollup {
            rel_path: "src".to_string(),
            mark: GitMark::Modified,
        }],
    );

    let git_of = |tree: &ExplorerState, path: &str| {
        tree.rows("")
            .into_iter()
            .find(|row| row.path == path)
            .map(|row| row.git)
    };
    assert_eq!(
        git_of(&tree, "src/main.rs"),
        Some(Some(GitStatus::Modified))
    );
    assert_eq!(git_of(&tree, "src"), Some(Some(GitStatus::Modified)));
    assert_eq!(git_of(&tree, "README.md"), Some(None));
}

#[test]
fn a_later_listing_keeps_the_marks() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing("", vec![dir("", "src")]));
    tree.toggle("src");
    tree.merge(listing("src", vec![file("src", "main.rs")]));
    tree.apply_git(
        1,
        &[git_entry(
            "src/main.rs",
            Some(GitPathChange::Untracked),
            None,
        )],
        &[GitRollup {
            rel_path: "src".to_string(),
            mark: GitMark::Untracked,
        }],
    );

    tree.merge(listing(
        "src",
        vec![file("src", "main.rs"), file("src", "lib.rs")],
    ));

    let git_of = |path: &str| {
        tree.rows("")
            .into_iter()
            .find(|row| row.path == path)
            .map(|row| row.git)
    };
    assert_eq!(git_of("src/main.rs"), Some(Some(GitStatus::Untracked)));
    assert_eq!(git_of("src/lib.rs"), Some(None));
    assert_eq!(git_of("src"), Some(Some(GitStatus::Untracked)));
}

#[test]
fn an_untracked_folder_marks_every_child() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing("", vec![dir("", "fresh"), file("", "README.md")]));
    tree.toggle("fresh");
    tree.merge(listing(
        "fresh",
        vec![file("fresh", "a.rs"), dir("fresh", "nested")],
    ));
    tree.toggle("fresh/nested");
    tree.merge(listing("fresh/nested", vec![file("fresh/nested", "b.rs")]));

    // libgit2 names untracked directories with a trailing slash; the host strips it, and so
    // does apply, so a mark named `fresh/` still lands on the `fresh` row.
    tree.apply_git(
        1,
        &[git_entry("fresh/", Some(GitPathChange::Untracked), None)],
        &[],
    );

    let git_of = |tree: &ExplorerState, path: &str| {
        tree.rows("")
            .into_iter()
            .find(|row| row.path == path)
            .map(|row| row.git)
    };
    assert_eq!(git_of(&tree, "fresh"), Some(Some(GitStatus::Untracked)));
    assert_eq!(
        git_of(&tree, "fresh/a.rs"),
        Some(Some(GitStatus::Untracked))
    );
    assert_eq!(
        git_of(&tree, "fresh/nested"),
        Some(Some(GitStatus::Untracked))
    );
    assert_eq!(
        git_of(&tree, "fresh/nested/b.rs"),
        Some(Some(GitStatus::Untracked))
    );
    assert_eq!(
        git_of(&tree, "README.md"),
        Some(None),
        "a sibling of the new folder is not untracked"
    );

    tree.merge(listing(
        "fresh",
        vec![
            file("fresh", "a.rs"),
            dir("fresh", "nested"),
            file("fresh", "c.rs"),
        ],
    ));
    assert_eq!(
        git_of(&tree, "fresh/c.rs"),
        Some(Some(GitStatus::Untracked)),
        "a listing after the map still inherits"
    );
}

#[test]
fn a_folder_that_only_contains_an_untracked_file_does_not_mark_its_other_children() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing("", vec![dir("", "src")]));
    tree.toggle("src");
    tree.merge(listing(
        "src",
        vec![file("src", "main.rs"), file("src", "lib.rs")],
    ));
    tree.apply_git(
        1,
        &[git_entry(
            "src/main.rs",
            Some(GitPathChange::Untracked),
            None,
        )],
        &[GitRollup {
            rel_path: "src".to_string(),
            mark: GitMark::Untracked,
        }],
    );

    let git_of = |path: &str| {
        tree.rows("")
            .into_iter()
            .find(|row| row.path == path)
            .map(|row| row.git)
    };
    assert_eq!(git_of("src"), Some(Some(GitStatus::Untracked)));
    assert_eq!(git_of("src/main.rs"), Some(Some(GitStatus::Untracked)));
    assert_eq!(git_of("src/lib.rs"), Some(None));
}

#[test]
fn a_stale_working_tree_is_discarded() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing("", vec![file("", "a.txt")]));
    assert!(tree.apply_git(
        2,
        &[git_entry("a.txt", Some(GitPathChange::Modified), None)],
        &[],
    ));
    assert!(!tree.apply_git(
        1,
        &[git_entry("a.txt", Some(GitPathChange::Untracked), None)],
        &[],
    ));
    assert_eq!(tree.rows("")[1].git, Some(GitStatus::Modified));
}

/// Backspace on macOS, Delete elsewhere: the row the keyboard is on is offered up for removal.
/// The tree removes nothing itself — a removal is the host's, and the window has a question to ask
/// first — so all this reports is which path was meant.
#[test]
fn the_delete_key_offers_the_row_the_keyboard_is_on() {
    let mut tree = listed();
    tree.set_cursor("justfile");

    assert_eq!(
        tree.press(ExplorerKey::Delete, ""),
        ExplorerPressed::Remove {
            path: "justfile".to_string(),
            is_dir: false,
        }
    );

    // A folder is offered the same way: what differs is only the wording of the question, which is
    // the window's business rather than the tree's.
    tree.set_cursor("src");
    assert_eq!(
        tree.press(ExplorerKey::Delete, ""),
        ExplorerPressed::Remove {
            path: "src".to_string(),
            is_dir: true,
        }
    );
}

/// The project's own row is not something the key can remove: its path names the folder the whole
/// tree describes, and the host refuses that too.
#[test]
fn the_delete_key_refuses_the_project_row() {
    let mut tree = listed();
    tree.set_cursor("");

    assert_eq!(
        tree.press(ExplorerKey::Delete, ""),
        ExplorerPressed::Ignored
    );
}

/// A row the host will not follow has nothing behind it to remove, the same reason Enter refuses
/// one.
#[test]
fn the_delete_key_refuses_a_row_the_host_will_not_follow() {
    let mut tree = ExplorerState::empty();
    tree.merge(listing("", vec![other("", "socket")]));
    tree.set_cursor("socket");

    assert_eq!(
        tree.press(ExplorerKey::Delete, ""),
        ExplorerPressed::Ignored
    );
}

/// With nothing under the cursor there is nothing to offer, and the key goes back to whoever else
/// wants it.
#[test]
fn the_delete_key_is_ignored_with_no_cursor() {
    let mut tree = listed();

    assert_eq!(
        tree.press(ExplorerKey::Delete, ""),
        ExplorerPressed::Ignored
    );
}
