//! Bookmarks: finding a line again, and writing one down twice.
//!
//! Both are arithmetic over text and a list — no window, no host, which is why the toggle is a
//! free function over a `Vec` rather than a method on the window.

use ubiq::state::dock::ChatId;
use ubiq::state::nav::{
    ANCHOR_CHARS, Anchored, Bookmark, Destination, Locus, View, resolve_anchor, toggle_mark,
};
use ubiq_proto::ids::ProjectId;

/// A file of numbered lines, so a line's text says which line it started as.
fn numbered(count: u32) -> String {
    (1..=count)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn mark(project: ProjectId, key: &str, line: u32) -> Bookmark {
    Bookmark {
        name: key.to_string(),
        dest: Destination {
            project,
            view: View::Ide { key: key.into() },
            locus: Some(Locus::Line { line }),
        },
        note: String::new(),
        anchor: None,
        adrift: false,
    }
}

/// The line is where it was said to be. Nothing is written and nothing moves.
#[test]
fn a_line_still_at_its_number_is_exact() {
    assert_eq!(
        resolve_anchor(&numbered(20), 7, "line 7"),
        Anchored::Exact(7)
    );
}

/// The file grew above the bookmark, so the line is a few numbers down from where it was.
#[test]
fn a_line_edited_above_is_found_and_moved() {
    let text = format!("new\nnew\nnew\n{}", numbered(20));
    assert_eq!(resolve_anchor(&text, 7, "line 7"), Anchored::Moved(10));
}

/// The line is gone. It is not repaired: the bookmark says it lost its place, at the number it
/// still holds.
#[test]
fn a_line_that_is_gone_is_adrift() {
    assert_eq!(
        resolve_anchor(&numbered(20), 7, "a line nobody wrote"),
        Anchored::Adrift(7)
    );
}

/// A number the file no longer reaches has no neighbourhood, so the whole file is searched — and
/// failing that, the bookmark comes to rest on the last line there is.
#[test]
fn a_number_past_the_end_scans_the_whole_file() {
    assert_eq!(
        resolve_anchor(&numbered(20), 500, "line 3"),
        Anchored::Moved(3)
    );
    assert_eq!(
        resolve_anchor(&numbered(20), 500, "gone"),
        Anchored::Adrift(20)
    );
}

/// The scan reaches exactly two hundred lines either way, and stops there. A line that has moved
/// further than that is a different file, not a moved bookmark.
#[test]
fn the_scan_reaches_two_hundred_lines_and_no_further() {
    let text = numbered(1000);
    assert_eq!(resolve_anchor(&text, 500, "line 300"), Anchored::Moved(300));
    assert_eq!(resolve_anchor(&text, 500, "line 700"), Anchored::Moved(700));
    assert_eq!(
        resolve_anchor(&text, 500, "line 299"),
        Anchored::Adrift(500)
    );
    assert_eq!(
        resolve_anchor(&text, 500, "line 701"),
        Anchored::Adrift(500)
    );
}

/// A line that appears twice does not have its bookmark stolen by the other copy: the number is
/// trusted before anything is searched for.
#[test]
fn a_duplicated_line_keeps_the_number_it_was_given() {
    let text = "same\nfiller\nfiller\nsame\n";
    assert_eq!(resolve_anchor(text, 4, "same"), Anchored::Exact(4));
    assert_eq!(resolve_anchor(text, 1, "same"), Anchored::Exact(1));
}

/// **Both sides of the comparison are capped.** The anchor was cut to 120 characters when it was
/// stored; a line longer than that would never match its own anchor if the line were not cut too.
#[test]
fn a_long_line_matches_its_capped_anchor() {
    let long: String = std::iter::repeat_n('x', ANCHOR_CHARS + 80).collect();
    let anchor: String = long.chars().take(ANCHOR_CHARS).collect();
    let text = format!("first\n{long}\nthird\n");

    assert_eq!(resolve_anchor(&text, 2, &anchor), Anchored::Exact(2));
    assert_eq!(
        resolve_anchor(&format!("added\n{text}"), 2, &anchor),
        Anchored::Moved(3)
    );
}

/// The anchor is the trimmed line, so indentation changing is not the line changing.
#[test]
fn indentation_is_not_part_of_the_line() {
    assert_eq!(
        resolve_anchor("a\n        let x = 1;\nb\n", 2, "let x = 1;"),
        Anchored::Exact(2)
    );
}

/// Writing a place down twice leaves the list as it was found.
#[test]
fn toggling_twice_is_the_list_unchanged() {
    let project = ProjectId::generate();
    let mut marks = vec![mark(project, "README.md", 1)];
    let before = marks.clone();

    let one = mark(project, "src/main.rs", 42);
    toggle_mark(&mut marks, one.clone());
    assert_eq!(marks.len(), 2);
    toggle_mark(&mut marks, one);
    assert_eq!(marks, before);
}

/// The same file at a different line is a different place, so it is a second bookmark rather than
/// the removal of the first.
#[test]
fn another_line_of_one_file_is_another_bookmark() {
    let project = ProjectId::generate();
    let mut marks = Vec::new();
    toggle_mark(&mut marks, mark(project, "src/main.rs", 10));
    toggle_mark(&mut marks, mark(project, "src/main.rs", 12));
    assert_eq!(marks.len(), 2);
}

/// A chat's id is minted by the window that drew it, so it is never written down.
#[test]
fn only_a_chat_refuses_to_be_written_down() {
    let project = ProjectId::generate();
    assert!(
        !Destination::new(
            project,
            View::Chat {
                chat: ChatId::generate()
            }
        )
        .persistable()
    );
    assert!(Destination::new(project, View::Control).persistable());
    assert!(
        Destination::new(
            project,
            View::Ide {
                key: "README.md".into()
            }
        )
        .persistable()
    );
}
