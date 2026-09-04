//! The vim command set, driven the way a user drives it: keys in, buffer and cursor out.
//!
//! No window, no entity, no gpui — the engine is a pure function over a `&str` and a byte range,
//! which is the whole reason it lives in `state/` rather than in the driver.
//!
//! A fixture writes the buffer with `|` where the cursor is, so a case reads as what the user
//! would see. `run` applies the effects the way `app/vim.rs` does: a `Replace` sets the selection
//! and then overwrites it, and the caret lands at the end of what was written.

use ubiq::state::vim::{Doc, Effect, Key, VimMode, VimState, step};

/// Split `a|bc` into the buffer and the cursor offset.
fn parse(marked: &str) -> (String, usize) {
    let at = marked
        .find('|')
        .expect("the fixture marks the cursor with |");
    (marked.replace('|', ""), at)
}

fn mark(text: &str, at: usize) -> String {
    format!("{}|{}", &text[..at], &text[at..])
}

/// Type `keys` into `marked` and return the buffer with the cursor marked again.
///
/// Every character is one keystroke. `<esc>` is Escape and `<cr>` is Enter, because neither has a
/// character to type.
fn run(marked: &str, keys: &str) -> String {
    let (mut text, at) = parse(marked);
    let mut st = VimState {
        mode: VimMode::Normal,
        ..VimState::default()
    };
    let mut sel = at..at;

    for key in split(keys) {
        let effects = step(
            &mut st,
            &key,
            Doc {
                text: &text,
                sel: sel.clone(),
            },
        );
        for effect in effects {
            match effect {
                Effect::Select(range) => sel = range,
                Effect::Replace(range, with) => {
                    let at = range.start + with.len();
                    text.replace_range(range, &with);
                    sel = at..at;
                }
                // The window-level effects are the driver's; `run` is about the buffer.
                Effect::Undo | Effect::Redo | Effect::Yank(_) => {}
                Effect::Save | Effect::Close { .. } => {}
            }
        }
    }
    mark(&text, sel.start)
}

fn split(keys: &str) -> Vec<Key> {
    let mut out = Vec::new();
    let mut rest = keys;
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix("<esc>") {
            out.push(Key::new("escape"));
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("<cr>") {
            out.push(Key::new("enter"));
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("<c-r>") {
            out.push(Key::ctrl("r"));
            rest = tail;
        } else {
            let c = rest.chars().next().unwrap();
            out.push(Key::new(&c.to_string()));
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

// --- motions -------------------------------------------------------------------------------------

#[test]
fn hjkl_move_by_one() {
    assert_eq!(run("a|bc", "l"), "ab|c");
    assert_eq!(run("ab|c", "h"), "a|bc");
    assert_eq!(run("one\nt|wo", "k"), "o|ne\ntwo");
    assert_eq!(run("o|ne\ntwo", "j"), "one\nt|wo");
}

#[test]
fn h_and_l_stay_on_their_line() {
    assert_eq!(run("|abc\ndef", "hhh"), "|abc\ndef");
    // The Normal-mode caret stops on the last character, never on the newline past it.
    assert_eq!(run("a|bc\ndef", "lll"), "ab|c\ndef");
}

#[test]
fn a_count_repeats_a_motion() {
    assert_eq!(run("|abcdef", "3l"), "abc|def");
}

/// Proposal §11: a count larger than the buffer stops at the last line rather than running off.
#[test]
fn a_count_past_the_end_stops_at_the_end() {
    assert_eq!(run("|one\ntwo\nthree", "9j"), "one\ntwo\n|three");
}

#[test]
fn j_and_k_keep_the_preferred_column() {
    // Down through a short line and out the other side comes back to where it started.
    assert_eq!(run("aaaa|aa\nbb\ncccccc", "jj"), "aaaaaa\nbb\ncccc|cc");
}

#[test]
fn word_motions_walk_by_class() {
    assert_eq!(run("|foo bar", "w"), "foo |bar");
    assert_eq!(run("|foo(bar)", "w"), "foo|(bar)");
    assert_eq!(run("foo |bar", "b"), "|foo bar");
    assert_eq!(run("|foo bar", "e"), "fo|o bar");
}

#[test]
fn line_ends_and_first_non_blank() {
    assert_eq!(run("ab|c\n", "0"), "|abc\n");
    assert_eq!(run("|  abc", "^"), "  |abc");
    assert_eq!(run("|abc", "$"), "ab|c");
}

#[test]
fn gg_and_g_reach_the_ends() {
    assert_eq!(run("one\ntwo\nthr|ee", "gg"), "|one\ntwo\nthree");
    assert_eq!(run("|one\ntwo\nthree", "G"), "one\ntwo\n|three");
    assert_eq!(run("|one\ntwo\nthree", "2G"), "one\n|two\nthree");
}

#[test]
fn f_and_t_find_on_the_line_only() {
    assert_eq!(run("|foo bar", "fb"), "foo |bar");
    assert_eq!(run("|foo bar", "tb"), "foo| bar");
    assert_eq!(run("|foo\nbar", "fb"), "|foo\nbar");
}

#[test]
fn percent_jumps_to_the_matching_bracket() {
    assert_eq!(run("f|oo(bar)", "%"), "foo(bar|)");
    assert_eq!(run("foo(bar|)", "%"), "foo|(bar)");
}

// --- editing --------------------------------------------------------------------------------------

#[test]
fn x_deletes_forwards_and_capital_x_backwards() {
    assert_eq!(run("a|bc", "x"), "a|c");
    assert_eq!(run("a|bc", "2x"), "a|");
    assert_eq!(run("ab|c", "X"), "a|c");
}

#[test]
fn dd_takes_the_whole_line() {
    assert_eq!(run("one\nt|wo\nthree", "dd"), "one\n|three");
}

/// Proposal §11: `dd` on the last line takes the newline in front of it, not one behind.
#[test]
fn dd_on_the_last_line_leaves_no_empty_line() {
    assert_eq!(run("one\nt|wo", "dd"), "one|");
}

#[test]
fn d_takes_a_motion() {
    assert_eq!(run("|foo bar", "dw"), "|bar");
    assert_eq!(run("foo| bar", "d$"), "foo|");
    assert_eq!(run("|abc", "dl"), "|bc");
}

#[test]
fn capital_d_and_c_reach_the_end_of_the_line() {
    assert_eq!(run("ab|cdef\nx", "D"), "ab|\nx");
}

#[test]
fn cc_clears_the_line_and_keeps_it() {
    assert_eq!(run("one\nt|wo\nthree", "cc"), "one\n|\nthree");
}

#[test]
fn cc_keeps_the_indentation() {
    assert_eq!(run("    t|wo\n", "cc"), "    |\n");
}

#[test]
fn o_and_capital_o_open_a_line_with_the_indent() {
    assert_eq!(run("  a|bc", "o"), "  abc\n  |");
    assert_eq!(run("  a|bc", "O"), "  |\n  abc");
}

#[test]
fn yank_then_paste() {
    assert_eq!(run("|foo bar", "ywP"), "foo |foo bar");
    assert_eq!(run("o|ne\ntwo", "yyp"), "one\n|one\ntwo");
}

#[test]
fn join_pulls_the_next_line_up() {
    assert_eq!(run("on|e\n   two", "J"), "one| two");
}

#[test]
fn tilde_flips_the_case_and_moves_on() {
    assert_eq!(run("|abc", "~"), "A|bc");
}

#[test]
fn indent_and_outdent_a_line() {
    assert_eq!(run("a|bc", ">>"), "|    abc");
    assert_eq!(run("    a|bc", "<<"), "|abc");
}

// --- modes ----------------------------------------------------------------------------------------

#[test]
fn escape_from_insert_steps_back_one() {
    let (text, at) = parse("ab|c");
    let mut st = VimState {
        mode: VimMode::Insert,
        ..VimState::default()
    };
    let effects = step(
        &mut st,
        &Key::new("escape"),
        Doc {
            text: &text,
            sel: at..at,
        },
    );
    assert_eq!(st.mode, VimMode::Normal);
    assert_eq!(effects, vec![Effect::Select(1..1)]);
}

#[test]
fn escape_from_insert_never_leaves_the_line() {
    let (text, at) = parse("abc\n|def");
    let mut st = VimState {
        mode: VimMode::Insert,
        ..VimState::default()
    };
    let effects = step(
        &mut st,
        &Key::new("escape"),
        Doc {
            text: &text,
            sel: at..at,
        },
    );
    assert_eq!(effects, vec![Effect::Select(4..4)]);
}

#[test]
fn insert_commands_land_where_vim_puts_them() {
    let cases = [("i", 1), ("a", 2), ("I", 0), ("A", 3)];
    for (keys, at) in cases {
        let (text, cur) = parse("a|bc");
        let mut st = VimState {
            mode: VimMode::Normal,
            ..VimState::default()
        };
        let effects = step(
            &mut st,
            &Key::new(keys),
            Doc {
                text: &text,
                sel: cur..cur,
            },
        );
        assert_eq!(st.mode, VimMode::Insert, "{keys}");
        assert_eq!(effects, vec![Effect::Select(at..at)], "{keys}");
    }
}

/// Proposal §11: a half-typed operator abandoned with Escape deletes nothing.
#[test]
fn escape_cancels_a_pending_operator() {
    assert_eq!(run("fo|o bar", "d<esc>"), "fo|o bar");
}

/// Proposal §11: Escape in Normal mode with nothing pending is not vim's to swallow — the driver
/// asks `claims` before it intercepts, so the modal underneath still closes.
#[test]
fn escape_with_nothing_pending_is_not_claimed() {
    let st = VimState {
        mode: VimMode::Normal,
        ..VimState::default()
    };
    assert!(!st.claims(&Key::new("escape")));

    let mut pending = VimState {
        mode: VimMode::Normal,
        ..VimState::default()
    };
    pending.pending.push('d');
    assert!(pending.claims(&Key::new("escape")));
}

#[test]
fn insert_mode_claims_nothing_but_escape() {
    let st = VimState {
        mode: VimMode::Insert,
        ..VimState::default()
    };
    assert!(st.claims(&Key::new("escape")));
    assert!(!st.claims(&Key::new("d")));
    assert!(!st.claims(&Key::new("enter")));
}

// --- visual and text objects ------------------------------------------------------------------------

#[test]
fn visual_selects_and_deletes() {
    assert_eq!(run("|foo bar", "vlld"), "| bar");
}

#[test]
fn visual_line_takes_whole_lines() {
    assert_eq!(run("one\nt|wo\nthree", "Vd"), "one\n|three");
}

#[test]
fn text_objects_reach_inside_and_around() {
    assert_eq!(run("foo b|ar baz", "diw"), "foo | baz");
    assert_eq!(run("foo b|ar baz", "daw"), "foo |baz");
    assert_eq!(run("say \"he|llo\" now", "di\""), "say \"|\" now");
    assert_eq!(run("f(a|rg)", "di("), "f(|)");
    assert_eq!(run("f(a|rg)", "da("), "f|");
}

/// Proposal §11: `ciw` where there is no word cancels and moves nothing.
#[test]
fn ciw_on_nothing_does_nothing() {
    assert_eq!(run("|", "ciw"), "|");
}

#[test]
fn an_operator_survives_a_count() {
    assert_eq!(run("|one\ntwo\nthree\nfour", "2dd"), "|three\nfour");
}

// --- search ------------------------------------------------------------------------------------------

#[test]
fn search_jumps_to_the_next_match() {
    assert_eq!(run("|alpha beta alpha", "/beta<cr>"), "alpha |beta alpha");
}

#[test]
fn n_repeats_the_search_and_wraps() {
    assert_eq!(run("|a x a x a", "/x<cr>n"), "a x a |x a");
    assert_eq!(run("|a x a x a", "/x<cr>nn"), "a |x a x a");
}

#[test]
fn star_searches_the_word_under_the_cursor() {
    assert_eq!(run("f|oo bar foo", "*"), "foo bar |foo");
}

/// `*` means the word, not any identifier it is a piece of: `foobar` is not a match for `foo`.
#[test]
fn star_matches_whole_words_only() {
    assert_eq!(run("f|oo foobar foo", "*"), "foo foobar |foo");
    assert_eq!(run("f|oo foobar", "*"), "|foo foobar");
}

/// `#` is `*` the other way, and wraps the same.
#[test]
fn hash_searches_backwards() {
    assert_eq!(run("foo bar f|oo", "#"), "|foo bar foo");
}

/// A pattern typed at `/` means exactly what was typed, so it still matches inside a word.
#[test]
fn a_typed_search_is_not_whole_word() {
    assert_eq!(run("|a foobar", "/foo<cr>"), "a |foobar");
}

/// The word under the cursor is found from a blank too, the way vim's `*` takes the next one.
#[test]
fn star_from_a_blank_takes_the_next_word() {
    assert_eq!(run("|  foo bar foo", "*"), "  foo bar |foo");
}

#[test]
fn an_abandoned_search_leaves_the_cursor_alone() {
    assert_eq!(run("|alpha beta", "/beta<esc>"), "|alpha beta");
}

// --- the ex commands -------------------------------------------------------------------------------

/// Drive `keys` from Normal mode and return the effects the last one produced.
fn effects(marked: &str, keys: &str) -> Vec<Effect> {
    let (text, at) = parse(marked);
    let mut st = VimState {
        mode: VimMode::Normal,
        ..VimState::default()
    };
    let sel = at..at;
    let mut last = Vec::new();
    for key in split(keys) {
        last = step(
            &mut st,
            &key,
            Doc {
                text: &text,
                sel: sel.clone(),
            },
        );
    }
    last
}

#[test]
fn colon_w_saves() {
    assert_eq!(effects("a|bc", ":w<cr>"), vec![Effect::Save]);
}

#[test]
fn colon_q_closes_the_editor() {
    assert_eq!(
        effects("a|bc", ":q<cr>"),
        vec![Effect::Close { discard: false }]
    );
}

/// `:qa` closes and throws the edit away. `:q!` is the same thing spelled the way vim spells it.
#[test]
fn colon_qa_closes_without_saving() {
    let discard = vec![Effect::Close { discard: true }];
    assert_eq!(effects("a|bc", ":qa<cr>"), discard);
    assert_eq!(effects("a|bc", ":q!<cr>"), discard);
}

#[test]
fn colon_wq_saves_then_closes() {
    assert_eq!(
        effects("a|bc", ":wq<cr>"),
        vec![Effect::Save, Effect::Close { discard: false }]
    );
}

#[test]
fn an_unknown_ex_command_does_nothing() {
    assert!(effects("a|bc", ":nope<cr>").is_empty());
    assert!(effects("a|bc", ":<cr>").is_empty());
}

#[test]
fn an_abandoned_ex_line_does_nothing() {
    assert_eq!(run("a|bc", ":w<esc>"), "a|bc");
}

#[test]
fn the_ex_line_is_what_the_status_bar_shows() {
    let (text, at) = parse("a|bc");
    let mut st = VimState {
        mode: VimMode::Normal,
        ..VimState::default()
    };
    for key in split(":wq") {
        step(
            &mut st,
            &key,
            Doc {
                text: &text,
                sel: at..at,
            },
        );
    }
    assert_eq!(st.label(), ":wq");
}

/// Backspacing past the lead closes the line rather than leaving an empty `:` armed.
#[test]
fn backspacing_off_the_ex_lead_closes_the_line() {
    let (text, at) = parse("a|bc");
    let mut st = VimState {
        mode: VimMode::Normal,
        ..VimState::default()
    };
    for key in [
        Key::new(":"),
        Key::new("w"),
        Key::new("backspace"),
        Key::new("backspace"),
    ] {
        step(
            &mut st,
            &key,
            Doc {
                text: &text,
                sel: at..at,
            },
        );
    }
    assert!(st.typing.is_none());
    assert_eq!(st.label(), "NORMAL");
}

/// Ex commands are Normal-mode only: in Insert mode a `:` is a colon.
#[test]
fn a_colon_in_insert_mode_is_just_a_colon() {
    let st = VimState {
        mode: VimMode::Insert,
        ..VimState::default()
    };
    assert!(!st.claims(&Key::new(":")));
}
