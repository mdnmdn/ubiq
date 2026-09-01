//! The file picker's rules, none of which need a frame.
//!
//! **What the dialog shows is a function of the request and the filter**, so every claim the screen
//! makes about it — a folders-only picker that lists no files, a prefilter that never hides a
//! folder, a list that says which folder each match came from, a single pick that replaces rather
//! than accumulates — is checked here against the state rather than against a picture of it.
//!
//! The fixture is the sink's own tree, because a picker tested against a tree written for the test
//! would stop testing the tree the application actually raises one over.

use ubiq::state::file_picker::{
    Commit, FilePickerState, MIN_HEIGHT, MIN_WIDTH, PickKind, PickerCount, PickerKey, PickerNode,
    PickerOwner, PickerRequest, PickerRow, PickerView, Pressed, matches_glob, size_label,
};
use ubiq::state::sink::picker_tree;

fn request() -> PickerRequest {
    PickerRequest::new(PickerOwner::Sink, "Select documentation files")
}

fn open(request: PickerRequest, view: PickerView) -> FilePickerState {
    FilePickerState::open(request, picker_tree(), view)
}

fn names(rows: &[PickerRow]) -> Vec<&str> {
    rows.iter().map(|row| row.name.as_str()).collect()
}

fn paths(rows: &[PickerRow]) -> Vec<&str> {
    rows.iter().map(|row| row.path.as_str()).collect()
}

// ── what a dialog opens on ──────────────────────────────────────────

/// The top of what is shown opens with the dialog. A picker whose only row is a shut folder makes
/// the user click once before it has shown them anything.
#[test]
fn a_picker_opens_with_the_folder_it_is_rooted_at_already_open() {
    let picker = open(request(), PickerView::Tree);
    let rows = picker.rows();

    assert_eq!(rows[0].name, "agent-manager");
    assert!(rows[0].expanded, "the root opened shut");
    assert!(names(&rows).contains(&"README.md"), "{:?}", names(&rows));
    // One level, and no more: nothing below a shut folder is drawn.
    assert!(
        !names(&rows).contains(&"architecture.md"),
        "{:?}",
        names(&rows)
    );
}

/// A request naming a folder opens on that folder, and on nothing above it.
#[test]
fn a_rooted_picker_shows_that_folder_and_not_the_project() {
    let picker = open(request().root("docs"), PickerView::Tree);
    let rows = picker.rows();

    assert_eq!(rows[0].name, "docs");
    assert!(!paths(&rows).contains(&"README.md"), "{:?}", paths(&rows));
    assert!(
        names(&rows).contains(&"architecture.md"),
        "{:?}",
        names(&rows)
    );
}

/// A folder nobody handed in leaves the dialog showing everything. An empty picker would claim the
/// project is empty, which is a different — and false — statement.
#[test]
fn a_root_that_is_not_there_opens_on_the_whole_forest() {
    let picker = open(request().root("does/not/exist"), PickerView::Tree);
    assert_eq!(picker.rows()[0].name, "agent-manager");
}

// ── the two arrangements ────────────────────────────────────────────

/// The tree draws what has been opened. Opening a folder draws what is inside it, and shutting it
/// takes it away again.
#[test]
fn the_tree_draws_the_folders_that_have_been_opened() {
    let mut picker = open(request(), PickerView::Tree);
    assert!(!names(&picker.rows()).contains(&"conventions.md"));

    picker.toggle_folder("docs");
    let rows = picker.rows();
    assert!(
        names(&rows).contains(&"conventions.md"),
        "{:?}",
        names(&rows)
    );
    // Its own folders stay shut: a listing is one level, not a subtree.
    assert!(
        !names(&rows).contains(&"0001-worktrees.md"),
        "{:?}",
        names(&rows)
    );

    picker.toggle_folder("docs");
    assert!(!names(&picker.rows()).contains(&"conventions.md"));
}

/// The list is every match under the root, flat, whatever is open — and each row says which folder
/// it came from, because a flat list of names is ambiguous the moment two folders agree on one.
#[test]
fn the_list_is_flat_and_says_which_folder_each_row_came_from() {
    let picker = open(request(), PickerView::List);
    let rows = picker.rows();

    assert!(rows.iter().all(|row| row.depth == 0), "the list indented");
    assert!(
        rows.iter().all(|row| !row.is_dir),
        "a folder in a file list"
    );

    let deep = rows
        .iter()
        .find(|row| row.name == "0001-worktrees.md")
        .expect("a file nothing was opened to reach");
    assert_eq!(deep.trailing, "docs/adr");

    let top = rows
        .iter()
        .find(|row| row.name == "README.md")
        .expect("the file at the top");
    assert_eq!(top.trailing, ".", "a file at the root says so");
}

/// Both views answer the same question, so what was picked in one is picked in the other.
#[test]
fn a_pick_survives_the_toggle_between_the_views() {
    let mut picker = open(request(), PickerView::List);
    picker.pick("docs/architecture.md");

    picker.set_view(PickerView::Tree);
    picker.toggle_folder("docs");

    let row = picker
        .rows()
        .into_iter()
        .find(|row| row.path == "docs/architecture.md")
        .expect("the file in the tree");
    assert!(row.selected, "the pick did not survive the toggle");
}

// ── what the request hides ──────────────────────────────────────────

/// A folders-only picker draws no files at all: a row that cannot be the answer and leads nowhere
/// is noise in a dialog whose whole job is to be scanned.
#[test]
fn a_folders_picker_draws_no_files_in_either_view() {
    let tree = open(request().kind(PickKind::Folders), PickerView::Tree);
    assert!(
        tree.rows().iter().all(|row| row.is_dir),
        "{:?}",
        names(&tree.rows())
    );
    assert!(tree.rows().iter().all(|row| row.pickable));

    let list = open(request().kind(PickKind::Folders), PickerView::List);
    let rows = list.rows();
    assert!(rows.iter().all(|row| row.is_dir), "{:?}", names(&rows));
    assert!(paths(&rows).contains(&"docs/adr"), "{:?}", paths(&rows));
    // The folder the dialog is rooted at is the ground, never a row of its own in the flat view.
    assert!(!paths(&rows).contains(&""), "the root listed itself");
}

/// The prefilter cuts files down and never hides a folder — a folder it hid would take the files
/// under it with it.
#[test]
fn the_prefilter_reaches_files_and_never_folders() {
    let picker = open(request().pattern(Some("*.md")), PickerView::List);
    let rows = picker.rows();

    assert!(names(&rows).contains(&"README.md"));
    assert!(
        !names(&rows).contains(&"package.json"),
        "{:?}",
        names(&rows)
    );

    let mut tree = open(request().pattern(Some("*.md")), PickerView::Tree);
    tree.toggle_folder("src-tauri");
    let rows = tree.rows();
    assert!(names(&rows).contains(&"src-tauri"), "a folder was hidden");
    assert!(
        !names(&rows).contains(&"tauri.conf.json"),
        "{:?}",
        names(&rows)
    );
}

/// A filter finds rather than prunes: every folder is walked while one is typed, and a folder with
/// nothing under it drops out instead of drawing as empty.
#[test]
fn a_filter_finds_through_shut_folders_and_drops_the_empty_ones() {
    let mut picker = open(request(), PickerView::Tree);
    picker.set_filter("worktrees".to_string());
    let rows = picker.rows();

    assert!(
        paths(&rows).contains(&"docs/adr/0001-worktrees.md"),
        "{:?}",
        paths(&rows)
    );
    assert!(names(&rows).contains(&"adr"), "the way in went missing");
    assert!(!names(&rows).contains(&"src"), "{:?}", names(&rows));
    assert!(
        !names(&rows).contains(&"README.md"),
        "an unmatched file stayed"
    );
}

// ── picking ─────────────────────────────────────────────────────────

/// Several picks accumulate and untick, in the order they were made — which is the order they are
/// handed back in.
#[test]
fn a_multiple_pick_accumulates_and_unticks() {
    let mut picker = open(request(), PickerView::List);

    assert!(
        !picker.pick("docs/conventions.md"),
        "a multiple pick closed"
    );
    picker.pick("docs/architecture.md");
    assert_eq!(
        picker.picked(),
        ["docs/conventions.md", "docs/architecture.md"]
    );
    assert_eq!(picker.tally(), "2 selected");
    assert_eq!(picker.confirm_label(), "Add 2");

    picker.pick("docs/conventions.md");
    assert_eq!(picker.picked(), ["docs/architecture.md"]);
}

/// A single pick replaces what was there. That is what makes it single: two ticks in a dialog that
/// takes one answer would leave the dialog to decide which of them it meant.
#[test]
fn a_single_pick_replaces_rather_than_accumulates() {
    let mut picker = open(request().count(PickerCount::Single), PickerView::List);

    assert!(!picker.pick("README.md"), "a pick on the button closed");
    picker.pick("package.json");
    assert_eq!(picker.picked(), ["package.json"]);
    assert_eq!(picker.confirm_label(), "Select");
}

/// A single pick asked to be final on the click says so, and only then. A multiple pick never does,
/// however it was asked for: there is no click that means "and that is all of them".
#[test]
fn only_a_single_pick_can_be_final_on_the_click() {
    let mut on_click = open(
        request().count(PickerCount::Single).commit(Commit::OnClick),
        PickerView::List,
    );
    assert!(on_click.pick("README.md"), "the click was not final");

    let mut many = open(request().commit(Commit::OnClick), PickerView::List);
    assert!(!many.request.commits_on_click());
    assert!(!many.pick("README.md"), "a multiple pick closed on a click");
}

/// A folder that cannot be picked opens on its click — the only way into it in the tree — and one
/// that can be picked is chosen instead.
#[test]
fn a_click_opens_a_folder_it_cannot_pick_and_picks_one_it_can() {
    let mut files = open(request(), PickerView::Tree);
    files.click("docs");
    assert!(names(&files.rows()).contains(&"conventions.md"));
    assert!(
        files.picked().is_empty(),
        "a folder was picked by a file ask"
    );

    let mut folders = open(request().kind(PickKind::Folders), PickerView::Tree);
    folders.click("docs");
    assert_eq!(folders.picked(), ["docs"]);
    assert!(
        !names(&folders.rows()).contains(&"adr"),
        "the pick also opened it"
    );
}

/// Nothing chosen is nothing to hand back, whichever way the dialog was asked for.
#[test]
fn a_dialog_with_nothing_chosen_has_nothing_to_commit() {
    let mut picker = open(request(), PickerView::Tree);
    assert!(!picker.can_commit());
    assert_eq!(picker.confirm_label(), "Add");

    picker.pick("README.md");
    assert!(picker.can_commit());
}

// ── the keyboard ────────────────────────────────────────────────────

/// The keyboard starts on the first row, so the first arrow moves rather than makes a cursor
/// appear out of nowhere.
#[test]
fn a_dialog_opens_with_the_keyboard_on_its_first_row() {
    let picker = open(request(), PickerView::Tree);
    assert_eq!(picker.cursor(), Some(""), "not on the folder it opened at");
    assert_eq!(picker.cursor_index(), Some(0));
    assert!(picker.rows()[0].on_cursor);
}

/// Up and down walk the rows on screen and stop at the ends. A list that wrapped would lose the
/// user the moment they held the key down.
#[test]
fn the_arrows_walk_the_rows_and_stop_at_the_ends() {
    let mut picker = open(request(), PickerView::Tree);
    let rows = picker.rows();

    assert_eq!(picker.press(PickerKey::Down), Pressed::Moved);
    assert_eq!(picker.cursor(), Some(rows[1].path.as_str()));

    assert_eq!(picker.press(PickerKey::Up), Pressed::Moved);
    assert_eq!(picker.cursor(), Some(rows[0].path.as_str()));
    // Already at the top: the key is answered and nothing moves.
    picker.press(PickerKey::Up);
    assert_eq!(picker.cursor_index(), Some(0));

    for _ in 0..50 {
        picker.press(PickerKey::Down);
    }
    assert_eq!(picker.cursor_index(), Some(picker.rows().len() - 1));
}

/// Right opens the folder the cursor is on, then steps into it; left shuts it, then steps out.
#[test]
fn left_and_right_walk_the_tree_in_and_out() {
    let mut picker = open(request(), PickerView::Tree);
    picker.press(PickerKey::Down);
    assert_eq!(picker.cursor(), Some("docs"));

    assert_eq!(picker.press(PickerKey::Right), Pressed::Moved);
    assert!(names(&picker.rows()).contains(&"architecture.md"), "shut");
    assert_eq!(picker.cursor(), Some("docs"), "opening also moved");

    // Open already: the next press steps in, onto the first thing inside it.
    picker.press(PickerKey::Right);
    assert_eq!(picker.cursor(), Some("docs/architecture.md"));

    // A file has nowhere to go in, and the key goes back to whoever else wants it.
    assert_eq!(picker.press(PickerKey::Right), Pressed::Ignored);

    // From a file, left steps out to the folder holding it; from the folder, it shuts it.
    assert_eq!(picker.press(PickerKey::Left), Pressed::Moved);
    assert_eq!(picker.cursor(), Some("docs"));
    picker.press(PickerKey::Left);
    assert!(!names(&picker.rows()).contains(&"architecture.md"), "open");
    assert_eq!(picker.cursor(), Some("docs"), "shutting also moved");
}

/// The flat list has no depth to walk, so left and right mean nothing there — and saying so is what
/// gives the filter field its caret keys back.
#[test]
fn left_and_right_are_handed_back_in_the_flat_list() {
    let mut picker = open(request(), PickerView::List);
    assert_eq!(picker.press(PickerKey::Left), Pressed::Ignored);
    assert_eq!(picker.press(PickerKey::Right), Pressed::Ignored);
    // Up and down are the dialog's in both views.
    assert_eq!(picker.press(PickerKey::Down), Pressed::Moved);
}

/// Enter ticks a row where several may be chosen, and the dialog stays up: "and that is all of
/// them" is the other key.
#[test]
fn enter_ticks_a_row_when_several_may_be_picked() {
    let mut picker = open(request(), PickerView::List);
    assert_eq!(picker.press(PickerKey::Enter), Pressed::Moved);
    assert_eq!(picker.count(), 1);

    // And unticks it, exactly as the tick box does.
    picker.press(PickerKey::Enter);
    assert_eq!(picker.count(), 0);
}

/// Enter is the answer where one was asked for, whether or not the click was — a dialog raised for
/// a single file has nothing left to wait for once the file is on screen.
#[test]
fn enter_confirms_a_single_pick_however_the_click_was_asked_for() {
    for commit in [Commit::OnButton, Commit::OnClick] {
        let mut picker = open(
            request().count(PickerCount::Single).commit(commit),
            PickerView::List,
        );
        let first = picker.rows()[0].path.clone();

        assert_eq!(
            picker.press(PickerKey::Enter),
            Pressed::Commit,
            "{commit:?}"
        );
        assert_eq!(picker.picked(), [first]);
    }
}

/// Enter on a folder a files-only dialog cannot pick opens it. Nothing else it could mean is true.
#[test]
fn enter_opens_a_folder_that_cannot_be_picked() {
    let mut picker = open(request(), PickerView::Tree);
    picker.press(PickerKey::Down);
    assert_eq!(picker.press(PickerKey::Enter), Pressed::Moved);
    assert!(names(&picker.rows()).contains(&"architecture.md"));
    assert!(picker.picked().is_empty(), "a folder was picked");
}

/// Confirm hands back what has been ticked, and does nothing at all while nothing has been.
#[test]
fn confirm_hands_back_what_was_ticked_and_ignores_an_empty_dialog() {
    let mut picker = open(request(), PickerView::List);
    assert_eq!(picker.press(PickerKey::Confirm), Pressed::Ignored);

    picker.press(PickerKey::Enter);
    picker.press(PickerKey::Down);
    picker.press(PickerKey::Enter);
    assert_eq!(picker.press(PickerKey::Confirm), Pressed::Commit);
    assert_eq!(picker.count(), 2);
}

/// Escape is the way out, and it is the way out of every shape of dialog.
#[test]
fn escape_dismisses_whatever_the_dialog_was_asked_for() {
    for count in [PickerCount::Single, PickerCount::Multiple] {
        let mut picker = open(request().count(count).modal(true), PickerView::Tree);
        assert_eq!(picker.press(PickerKey::Dismiss), Pressed::Dismiss);
    }
}

/// A cursor left on a row that has gone is a cursor pointing at nothing, so filtering and switching
/// views both put it back on something that is there.
#[test]
fn the_cursor_is_put_back_on_a_row_that_is_still_drawn() {
    let mut picker = open(request(), PickerView::Tree);
    picker.press(PickerKey::Down);
    picker.press(PickerKey::Right);
    picker.press(PickerKey::Right);
    assert_eq!(picker.cursor(), Some("docs/architecture.md"));

    // A filter that keeps the row keeps the cursor on it.
    picker.set_filter("architecture".to_string());
    assert_eq!(picker.cursor(), Some("docs/architecture.md"));

    // One that does not moves it to the first row that survived.
    picker.set_filter("package".to_string());
    let first = picker.rows()[0].path.clone();
    assert_eq!(picker.cursor(), Some(first.as_str()));

    // The tree's root folder is no row at all in the flat list.
    picker.set_filter(String::new());
    picker.press(PickerKey::Up);
    assert_eq!(picker.cursor(), Some(""));
    picker.set_view(PickerView::List);
    assert!(picker.cursor_index().is_some(), "the cursor went nowhere");
}

/// The keyboard follows the mouse: an arrow after a click carries on from the row that was clicked.
#[test]
fn a_click_moves_the_keyboard_to_the_row_it_clicked() {
    let mut picker = open(request(), PickerView::List);
    let third = picker.rows()[2].path.clone();
    picker.click(&third);
    assert_eq!(picker.cursor(), Some(third.as_str()));
}

// ── how big it is ───────────────────────────────────────────────────

/// A resize stays inside what the dialog can be read at and what the window can hold.
#[test]
fn a_resize_is_clamped_to_the_dialog_and_to_the_window() {
    let mut picker = open(request(), PickerView::Tree);

    picker.resize(10.0, 10.0, (1600.0, 1200.0));
    assert_eq!((picker.width, picker.height), (MIN_WIDTH, MIN_HEIGHT));

    picker.resize(9_000.0, 9_000.0, (1600.0, 1200.0));
    assert_eq!((picker.width, picker.height), (1552.0, 1152.0));

    // A window smaller than the dialog's floor still gets a dialog: the floor wins, and the
    // element's own max keeps it on screen.
    picker.resize(9_000.0, 9_000.0, (200.0, 200.0));
    assert_eq!((picker.width, picker.height), (MIN_WIDTH, MIN_HEIGHT));
}

/// A drag is measured from where it started rather than from the last frame, so a pointer that
/// outruns the corner does not leave the dialog drifting behind it.
#[test]
fn a_corner_drag_is_measured_from_where_it_went_down() {
    let mut picker = open(request(), PickerView::Tree);
    let (width, height) = (picker.width, picker.height);

    assert!(!picker.drag_to((100.0, 100.0), (1600.0, 1200.0)), "no drag");

    picker.start_drag((500.0, 500.0));
    assert!(picker.is_resizing());
    picker.drag_to((520.0, 510.0), (1600.0, 1200.0));
    // The dialog is centred, so both of its edges move: it grows twice what the pointer did.
    assert_eq!(picker.width, width + 40.0);
    assert_eq!(picker.height, height + 20.0);

    // The same drag continued is still measured from the start, never accumulated.
    picker.drag_to((520.0, 510.0), (1600.0, 1200.0));
    assert_eq!(picker.width, width + 40.0);

    picker.end_drag();
    assert!(!picker.is_resizing());
    assert!(
        !picker.drag_to((900.0, 900.0), (1600.0, 1200.0)),
        "still on"
    );
}

// ── the small pure things ───────────────────────────────────────────

/// The prefilter is a handful of wildcards and nothing more, which is exactly what it claims to be.
#[test]
fn the_prefilter_understands_stars_and_question_marks_without_case() {
    assert!(matches_glob("*.md", "README.md"));
    assert!(matches_glob("*.MD", "readme.md"), "case leaked in");
    assert!(!matches_glob("*.md", "package.json"));
    assert!(matches_glob("Cargo.*", "Cargo.toml"));
    assert!(matches_glob("?ain.rs", "main.rs"));
    assert!(!matches_glob("?ain.rs", "domain.rs"));
    assert!(matches_glob("*", "anything at all"));
    assert!(matches_glob("*.*", "a.b"));
    assert!(!matches_glob("a*b", "ac"));
    assert!(matches_glob("a*b*c", "axxbyyc"));
}

/// A size is reported in the unit a person reads it in, and a folder reports nothing at all.
#[test]
fn a_size_is_reported_in_the_unit_it_is_read_in() {
    assert_eq!(size_label(None), "");
    assert_eq!(size_label(Some(512)), "512 B");
    assert_eq!(size_label(Some(12_400)), "12 KB");
    assert_eq!(size_label(Some(5_000_000)), "4 MB");
}

/// A file has no children and a folder always has the vector, empty or not — which is what tells
/// the dialog whether a row has a twisty before anything has been opened.
#[test]
fn a_folder_is_a_folder_even_when_there_is_nothing_in_it() {
    let empty = PickerNode::dir("target", "target", Vec::new());
    assert!(empty.is_dir());
    assert!(empty.children().is_empty());
    assert!(!PickerNode::file("main.rs", "src/main.rs", 10).is_dir());
}
