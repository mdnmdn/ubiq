//! The file tree: how a listing lands in it, what survives a re-listing, and how a remembered set
//! of open folders is restored one level at a time.
//!
//! All of it without a frame. The tree is pure state precisely so that these rules can be asserted
//! rather than clicked through.

use ubiq::state::explorer::{ExplorerState, NodeKind, Toggle};
use ubiq_proto::files::{DirEntry, DirListing, EntryKind};

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

fn names(tree: &ExplorerState) -> Vec<String> {
    tree.rows("").into_iter().map(|row| row.path).collect()
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

    // The match is reached even though its folder is shut, and nothing that did not match is drawn.
    let paths: Vec<String> = tree
        .rows("sessions")
        .into_iter()
        .map(|row| row.path)
        .collect();
    assert_eq!(paths, ["src/sessions.ts"]);

    // A folder that matches is drawn open, because its children are what the user is looking for.
    let rows = tree.rows("src");
    let paths: Vec<&str> = rows.iter().map(|row| row.path.as_str()).collect();
    assert_eq!(paths, ["src", "src/sessions.ts", "src/main.rs"]);
    assert!(rows[0].expanded);

    assert!(tree.rows("nothing-matches-this").is_empty());
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
    assert_eq!(rows.len(), 2);
    assert!(!rows[0].readable);
    assert!(!rows[0].is_dir, "there is nothing to expand it into");
    assert!(rows[1].readable);
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
