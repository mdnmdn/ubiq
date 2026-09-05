//! The destination value and the two line helpers the router puts a caret with.
//!
//! No window and no host: a destination is arithmetic over ids, and `offset_of_line` is arithmetic
//! over bytes. Both are total by contract, so every test here is as much about what they answer
//! past the end as about what they answer inside it.

use ubiq::state::editor::{Subject, tab_key};
use ubiq::state::nav::{
    Destination, Fate, History, Locus, View, line_of_slug, line_range, offset_of_line, range_for,
};
use ubiq_proto::files::DiffBase;
use ubiq_proto::ids::ProjectId;

const TEXT: &str = "one\ntwo\nthree\n";

#[test]
fn line_one_starts_at_the_beginning() {
    assert_eq!(offset_of_line(TEXT, 1), 0);
    // A zero is not a line, and answering the start is the total reading of it.
    assert_eq!(offset_of_line(TEXT, 0), 0);
}

#[test]
fn an_interior_line_starts_after_its_predecessors_newline() {
    assert_eq!(offset_of_line(TEXT, 2), 4);
    assert_eq!(offset_of_line(TEXT, 3), 8);
}

#[test]
fn a_line_past_the_end_is_the_end() {
    assert_eq!(offset_of_line(TEXT, 9), TEXT.len());
    assert_eq!(offset_of_line("", 4), 0);
}

#[test]
fn the_offsets_are_bytes_and_not_characters() {
    let text = "héllo — ok\nsecond\n";
    // Fourteen bytes and ten characters before the newline: a character count would land the
    // caret mid-scalar and panic on the slice.
    assert_eq!(offset_of_line(text, 2), text.find('\n').unwrap() + 1);
    assert!(text.is_char_boundary(offset_of_line(text, 2)));
    assert_eq!(line_range(text, 1, 1), 0..text.find('\n').unwrap());
}

#[test]
fn a_range_covers_whole_lines_without_the_last_newline() {
    assert_eq!(line_range(TEXT, 1, 1), 0..3);
    assert_eq!(line_range(TEXT, 2, 3), 4..13);
    // Reversed bounds are taken the right way round rather than answered empty.
    assert_eq!(line_range(TEXT, 3, 2), 4..13);
    // Past the end on either side is the end, and never a backwards range.
    assert_eq!(line_range(TEXT, 9, 9), TEXT.len()..TEXT.len());
    assert_eq!(line_range(TEXT, 2, 9), 4..TEXT.len());
}

#[test]
fn the_same_place_ignores_the_locus() {
    let project = ProjectId::generate();
    let here = Destination::new(
        project,
        View::Ide {
            key: "src/app.rs".into(),
        },
    );
    let there = Destination {
        locus: Some(Locus::Line { line: 200 }),
        ..here.clone()
    };
    assert!(here.same_place(&there));
    assert_ne!(here, there);
}

#[test]
fn a_file_and_its_diff_are_two_places() {
    let project = ProjectId::generate();
    let file = Destination::new(
        project,
        View::Ide {
            key: tab_key("src/app.rs", Subject::File),
        },
    );
    let diff = Destination::new(
        project,
        View::Ide {
            key: tab_key("src/app.rs", Subject::Diff(DiffBase::Head)),
        },
    );
    assert!(!file.same_place(&diff));
}

#[test]
fn the_same_view_in_two_projects_is_two_places() {
    let view = View::Explorer { path: "src".into() };
    let here = Destination::new(ProjectId::generate(), view.clone());
    let there = Destination::new(ProjectId::generate(), view);
    assert!(!here.same_place(&there));
}

// ── history ─────────────────────────────────────────────────────────

/// A destination in a made-up project, so a test can say which projects are live by hand.
fn at(project: ProjectId, key: &str) -> Destination {
    Destination::new(
        project,
        View::Ide {
            key: key.to_string(),
        },
    )
}

/// Everything is where this window can reach it.
fn all_here(_: &Destination) -> Fate {
    Fate::Here
}

#[test]
fn the_same_place_again_refreshes_the_locus_and_pushes_nothing() {
    let project = ProjectId::generate();
    let mut nav = History::default();
    nav.record(at(project, "src/app.rs"));
    let mut scrolled = at(project, "src/app.rs");
    scrolled.locus = Some(Locus::Line { line: 42 });
    nav.record(scrolled);

    assert_eq!(nav.entries.len(), 1);
    assert_eq!(nav.at, 0);
    assert_eq!(nav.current().unwrap().locus, Some(Locus::Line { line: 42 }));
}

#[test]
fn recording_after_a_back_truncates_the_forward_half() {
    let project = ProjectId::generate();
    let mut nav = History::default();
    for key in ["a", "b", "c"] {
        nav.record(at(project, key));
    }
    nav.back(&all_here);
    nav.record(at(project, "d"));

    assert_eq!(nav.entries.len(), 3);
    assert_eq!(nav.at, 2);
    assert_eq!(nav.current().unwrap().label(), "d");
}

#[test]
fn back_and_forward_move_the_cursor_and_push_nothing() {
    let project = ProjectId::generate();
    let mut nav = History::default();
    for key in ["a", "b", "c"] {
        nav.record(at(project, key));
    }

    assert_eq!(nav.back(&all_here).unwrap().label(), "b");
    assert_eq!(nav.back(&all_here).unwrap().label(), "a");
    assert!(nav.back(&all_here).is_none());
    assert_eq!(nav.forward(&all_here).unwrap().label(), "b");
    assert_eq!(nav.entries.len(), 3);
    assert_eq!(nav.at, 1);
}

#[test]
fn the_oldest_arrivals_fall_off_the_end() {
    let project = ProjectId::generate();
    let mut nav = History::default();
    for n in 0..70 {
        nav.record(at(project, &n.to_string()));
    }

    assert_eq!(nav.entries.len(), 64);
    assert_eq!(nav.entries[0].label(), "6");
    assert_eq!(nav.at, 63);
    assert_eq!(nav.current().unwrap().label(), "69");
}

#[test]
fn a_project_another_window_holds_is_stepped_over_and_kept() {
    let mine = ProjectId::generate();
    let theirs = ProjectId::generate();
    let mut nav = History::default();
    nav.record(at(mine, "a"));
    nav.record(at(theirs, "b"));
    nav.record(at(mine, "c"));

    let live = |dest: &Destination| {
        if dest.project == theirs {
            Fate::Elsewhere
        } else {
            Fate::Here
        }
    };
    // The other window's entry is skipped, not taken — nothing moves between windows.
    assert_eq!(nav.back(&live).unwrap().label(), "a");
    assert_eq!(nav.entries.len(), 3);
    assert_eq!(nav.at, 0);
    assert_eq!(nav.forward(&live).unwrap().label(), "c");
    assert_eq!(nav.at, 2);
}

#[test]
fn a_forgotten_project_is_dropped_and_the_cursor_still_points_at_the_place_it_did() {
    let mine = ProjectId::generate();
    let dead = ProjectId::generate();
    let live = |dest: &Destination| {
        if dest.project == dead {
            Fate::Gone
        } else {
            Fate::Here
        }
    };

    let mut nav = History::default();
    for dest in [at(mine, "a"), at(dead, "b"), at(mine, "c")] {
        nav.record(dest);
    }
    assert_eq!(nav.back(&live).unwrap().label(), "a");
    assert_eq!(nav.entries.len(), 2);
    assert_eq!(nav.at, 0);
    assert_eq!(nav.entries[1].label(), "c");

    // And the same going the other way: the cursor stays on what it was standing on.
    let mut nav = History::default();
    for dest in [at(mine, "a"), at(dead, "b"), at(mine, "c")] {
        nav.record(dest);
    }
    nav.at = 0;
    assert_eq!(nav.forward(&live).unwrap().label(), "c");
    assert_eq!(nav.entries.len(), 2);
    assert_eq!(nav.at, 1);
    assert_eq!(nav.entries[0].label(), "a");
}

#[test]
fn a_dead_end_is_dropped_and_the_press_answers_nothing() {
    let mine = ProjectId::generate();
    let dead = ProjectId::generate();
    let live = |dest: &Destination| {
        if dest.project == dead {
            Fate::Gone
        } else {
            Fate::Here
        }
    };

    let mut nav = History::default();
    nav.record(at(mine, "a"));
    nav.record(at(dead, "b"));
    nav.at = 0;
    assert!(nav.forward(&live).is_none());
    assert_eq!(nav.entries.len(), 1);
    assert_eq!(nav.at, 0);
    assert_eq!(nav.current().unwrap().label(), "a");

    let mut nav = History::default();
    nav.record(at(dead, "a"));
    nav.record(at(mine, "b"));
    assert!(nav.back(&live).is_none());
    assert_eq!(nav.entries.len(), 1);
    assert_eq!(nav.at, 0);
    assert_eq!(nav.current().unwrap().label(), "b");
}

#[test]
fn peek_names_the_neighbour_in_each_direction() {
    let project = ProjectId::generate();
    let mut nav = History::default();
    assert!(nav.peek(true).is_none());
    nav.record(at(project, "a"));
    nav.record(at(project, "b"));

    assert_eq!(nav.peek(true).unwrap().label(), "a");
    assert!(nav.peek(false).is_none());
    nav.back(&all_here);
    assert_eq!(nav.peek(false).unwrap().label(), "b");
}

const DOC: &str = "# Title\n\nprose\n\n## A Heading, With Punctuation\n\nmore\n\n### Repeated\n\nx\n\n## Repeated\n";

#[test]
fn a_heading_slug_finds_its_line() {
    assert_eq!(line_of_slug(DOC, "title"), Some(1));
    assert_eq!(line_of_slug(DOC, "a-heading-with-punctuation"), Some(5));
    // The link may be written as the heading was, not as the slug is.
    assert_eq!(line_of_slug(DOC, "A Heading, With Punctuation"), Some(5));
}

#[test]
fn a_repeated_heading_answers_the_first_and_an_absent_one_answers_nothing() {
    assert_eq!(line_of_slug(DOC, "repeated"), Some(9));
    assert_eq!(line_of_slug(DOC, "nowhere"), None);
}

#[test]
fn an_anchor_is_a_range_over_the_heading_it_names() {
    let range = range_for(
        DOC,
        &Locus::Anchor {
            slug: "title".into(),
        },
    )
    .unwrap();
    assert_eq!(&DOC[range], "# Title");
    // Dropped, never refused: a slug nothing matches puts no caret anywhere.
    assert_eq!(
        range_for(
            DOC,
            &Locus::Anchor {
                slug: "nowhere".into()
            }
        ),
        None
    );
}
