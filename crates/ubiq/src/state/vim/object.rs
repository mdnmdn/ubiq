//! Text objects: `iw`, `aw`, and the quoted and bracketed pairs.
//!
//! Each one answers with the byte range an operator should cover, or nothing when the cursor is
//! not inside anything of that kind — `ci"` outside a string does nothing rather than guessing.

use std::ops::Range;

use super::motion::{before, char_at, class, line_end, line_start, next, prev};

/// `iw` and `aw`. `around` takes the run of blanks after the word too, and falls back to the
/// blanks before it when there are none after — which is what makes `daw` at the end of a line
/// leave no double space behind.
pub fn word(text: &str, off: usize, around: bool) -> Option<Range<usize>> {
    let here = char_at(text, off)?;
    let kind = class(here);

    let mut start = off;
    while before(text, start).is_some_and(|c| class(c) == kind) {
        start = prev(text, start);
    }
    let mut end = off;
    while char_at(text, end).is_some_and(|c| class(c) == kind) {
        end = next(text, end);
    }
    if !around {
        return Some(start..end);
    }

    let mut after = end;
    while char_at(text, after).is_some_and(|c| c.is_whitespace() && c != '\n') {
        after = next(text, after);
    }
    if after > end {
        return Some(start..after);
    }
    let mut ahead = start;
    while before(text, ahead).is_some_and(|c| c.is_whitespace() && c != '\n') {
        ahead = prev(text, ahead);
    }
    Some(ahead..end)
}

/// `i"` / `a"` and the other quote characters. Scans the cursor's line only, because an unbalanced
/// quote elsewhere in the file would otherwise swallow half the buffer.
pub fn quoted(text: &str, off: usize, quote: char, around: bool) -> Option<Range<usize>> {
    let start_of_line = line_start(text, off);
    let end_of_line = line_end(text, off);

    let mut open = None;
    let mut probe = start_of_line;
    while probe < end_of_line {
        if char_at(text, probe) == Some(quote) {
            match open {
                None => open = Some(probe),
                Some(from) => {
                    if off <= probe {
                        return Some(if around {
                            from..next(text, probe)
                        } else {
                            next(text, from)..probe
                        });
                    }
                    open = None;
                }
            }
        }
        probe = next(text, probe);
    }
    None
}

/// `i(` / `a{` and the rest. Counts depth outwards from the cursor, so a nested pair picks the
/// innermost one the cursor is actually inside.
pub fn bracketed(
    text: &str,
    off: usize,
    open: char,
    close: char,
    around: bool,
) -> Option<Range<usize>> {
    let from = scan_back(text, off, open, close)?;
    let to = scan_forward(text, off, open, close)?;
    Some(if around {
        from..next(text, to)
    } else {
        next(text, from)..to
    })
}

fn scan_back(text: &str, off: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0i32;
    let mut probe = off;
    if char_at(text, probe) == Some(open) {
        return Some(probe);
    }
    while probe > 0 {
        probe = prev(text, probe);
        match char_at(text, probe) {
            Some(c) if c == close => depth += 1,
            Some(c) if c == open => {
                if depth == 0 {
                    return Some(probe);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

fn scan_forward(text: &str, off: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0i32;
    let mut probe = off;
    if char_at(text, probe) == Some(open) {
        probe = next(text, probe);
    }
    while probe < text.len() {
        match char_at(text, probe) {
            Some(c) if c == open => depth += 1,
            Some(c) if c == close => {
                if depth == 0 {
                    return Some(probe);
                }
                depth -= 1;
            }
            _ => {}
        }
        probe = next(text, probe);
    }
    None
}

/// The object a two-key sequence names — `iw`, `a"`, `i(` — or nothing when the second key is not
/// one this build knows.
pub fn resolve(text: &str, off: usize, around: bool, key: char) -> Option<Range<usize>> {
    match key {
        'w' | 'W' => word(text, off, around),
        '"' | '\'' | '`' => quoted(text, off, key, around),
        '(' | ')' | 'b' => bracketed(text, off, '(', ')', around),
        '[' | ']' => bracketed(text, off, '[', ']', around),
        '{' | '}' | 'B' => bracketed(text, off, '{', '}', around),
        '<' | '>' => bracketed(text, off, '<', '>', around),
        _ => None,
    }
}

/// Whether a key could start a text object, so `d` followed by `i` knows to wait rather than
/// cancel.
pub fn is_prefix(key: char) -> bool {
    matches!(key, 'i' | 'a')
}
