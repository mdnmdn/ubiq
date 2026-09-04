//! Motions: from a byte offset in a buffer to another byte offset.
//!
//! Everything here is a pure function over `&str`. Offsets are UTF-8 byte offsets into the buffer,
//! the same units the component's `selected_range()` speaks, and every step moves by characters so
//! a multi-byte character is never split.
//!
//! ponytail: `j` and `k` move by logical line, not display line. In a soft-wrapped buffer a long
//! line counts as one, which is what `nowrap` vim does and not what a wrapped editor looks like.
//! The upgrade needs the component's display map, which is not reachable from here.

use std::ops::Range;

/// How far an operator reaches when it is given a motion.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MotionKind {
    /// Up to but not including the target — `w`, `0`, `b`.
    Exclusive,
    /// Up to and including the character at the target — `e`, `f`, `%`.
    Inclusive,
    /// Whole lines, however far along them the two ends sit — `j`, `G`, `}`.
    Linewise,
}

/// Where a motion lands, and how much an operator over it covers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Move {
    pub to: usize,
    pub kind: MotionKind,
}

impl Move {
    pub fn exclusive(to: usize) -> Self {
        Self {
            to,
            kind: MotionKind::Exclusive,
        }
    }
    pub fn inclusive(to: usize) -> Self {
        Self {
            to,
            kind: MotionKind::Inclusive,
        }
    }
    pub fn linewise(to: usize) -> Self {
        Self {
            to,
            kind: MotionKind::Linewise,
        }
    }
}

/// Which of vim's three character classes a character belongs to. Word motions stop wherever the
/// class changes, which is what makes `w` walk off `foo` and onto `(`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Blank,
    Word,
    Punct,
}

pub fn class(c: char) -> Class {
    if c.is_whitespace() {
        Class::Blank
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

// --- character stepping ------------------------------------------------------------------------

/// The offset one character to the right, clamped to the end of the buffer.
pub fn next(text: &str, at: usize) -> usize {
    text[at..].chars().next().map_or(at, |c| at + c.len_utf8())
}

/// The offset one character to the left, clamped to the start of the buffer.
pub fn prev(text: &str, at: usize) -> usize {
    text[..at]
        .chars()
        .next_back()
        .map_or(at, |c| at - c.len_utf8())
}

/// The character at an offset, or none at the end of the buffer.
pub fn char_at(text: &str, off: usize) -> Option<char> {
    text[off..].chars().next()
}

/// The character before an offset.
pub fn before(text: &str, off: usize) -> Option<char> {
    text[..off].chars().next_back()
}

// --- lines -------------------------------------------------------------------------------------

/// The offset of the first character of the line `at` sits on.
pub fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |i| i + 1)
}

/// The offset just past the last character of the line, before its newline.
pub fn line_end(text: &str, at: usize) -> usize {
    text[at..].find('\n').map_or(text.len(), |i| at + i)
}

/// The first character on the line that is not a blank — vim's `^`.
pub fn first_non_blank(text: &str, at: usize) -> usize {
    let start = line_start(text, at);
    let end = line_end(text, at);
    let mut off = start;
    while off < end && char_at(text, off).is_some_and(|c| c.is_whitespace()) {
        off = next(text, off);
    }
    off
}

/// How many characters along its line an offset sits.
pub fn column(text: &str, at: usize) -> usize {
    text[line_start(text, at)..at].chars().count()
}

/// The offset `col` characters along the line starting at `start`, or the line's end.
pub fn at_column(text: &str, start: usize, col: usize) -> usize {
    let end = line_end(text, start);
    let mut off = start;
    for _ in 0..col {
        if off >= end {
            break;
        }
        off = next(text, off);
    }
    off
}

/// Move `count` lines down (positive) or up (negative), keeping `col`. Stops at the first or last
/// line rather than running off, which is what vim does with a count larger than the buffer.
pub fn vertical(text: &str, at: usize, delta: isize, col: usize) -> usize {
    let mut start = line_start(text, at);
    if delta >= 0 {
        for _ in 0..delta {
            let end = line_end(text, start);
            if end >= text.len() {
                break;
            }
            start = end + 1;
        }
    } else {
        for _ in 0..-delta {
            if start == 0 {
                break;
            }
            start = line_start(text, start - 1);
        }
    }
    at_column(text, start, col)
}

/// The start of the last line — vim's `G` with no count.
pub fn last_line(text: &str) -> usize {
    line_start(text, text.len())
}

/// The start of line `n`, counting from one — `G` and `gg` with a count.
pub fn nth_line(text: &str, n: u32) -> usize {
    let mut start = 0;
    for _ in 1..n.max(1) {
        let end = line_end(text, start);
        if end >= text.len() {
            break;
        }
        start = end + 1;
    }
    start
}

// --- words -------------------------------------------------------------------------------------

/// `w`: the start of the next word.
pub fn word_forward(text: &str, mut off: usize, count: u32) -> usize {
    for _ in 0..count.max(1) {
        let Some(here) = char_at(text, off) else {
            break;
        };
        let start = class(here);
        if start != Class::Blank {
            while char_at(text, off).is_some_and(|c| class(c) == start) {
                off = next(text, off);
            }
        }
        while char_at(text, off).is_some_and(|c| c.is_whitespace()) {
            off = next(text, off);
        }
    }
    off
}

/// `b`: the start of the current or previous word.
pub fn word_back(text: &str, mut off: usize, count: u32) -> usize {
    for _ in 0..count.max(1) {
        while before(text, off).is_some_and(|c| c.is_whitespace()) {
            off = prev(text, off);
        }
        let Some(c) = before(text, off) else { break };
        let start = class(c);
        while before(text, off).is_some_and(|c| class(c) == start) {
            off = prev(text, off);
        }
    }
    off
}

/// `e`: the last character of the current or next word.
pub fn word_end(text: &str, mut off: usize, count: u32) -> usize {
    for _ in 0..count.max(1) {
        off = next(text, off);
        while char_at(text, off).is_some_and(|c| c.is_whitespace()) {
            off = next(text, off);
        }
        let Some(here) = char_at(text, off) else {
            break;
        };
        let start = class(here);
        while char_at(text, next(text, off)).is_some_and(|c| class(c) == start) {
            off = next(text, off);
        }
    }
    off
}

/// The word under the cursor, for `*` and `#`, as a byte range.
///
/// A cursor sitting on a blank takes the next word on the line, the way vim's `*` does — and the
/// range is what says where that word starts, which is where the search has to begin so that `*`
/// lands on the *next* occurrence rather than on the word it was given.
pub fn word_at(text: &str, off: usize) -> Option<Range<usize>> {
    let mut here = off;
    while char_at(text, here).is_some_and(|c| class(c) != Class::Word) && here < line_end(text, off)
    {
        here = next(text, here);
    }
    if char_at(text, here).is_none_or(|c| class(c) != Class::Word) {
        return None;
    }
    let mut start = here;
    while before(text, start).is_some_and(|c| class(c) == Class::Word) {
        start = prev(text, start);
    }
    let mut end = here;
    while char_at(text, end).is_some_and(|c| class(c) == Class::Word) {
        end = next(text, end);
    }
    Some(start..end)
}

// --- paragraphs --------------------------------------------------------------------------------

fn blank_line(text: &str, start: usize) -> bool {
    text[start..line_end(text, start)].trim().is_empty()
}

/// `}`: the next blank line, or the end of the buffer.
pub fn paragraph_forward(text: &str, mut off: usize, count: u32) -> usize {
    for _ in 0..count.max(1) {
        let mut start = line_start(text, off);
        loop {
            let end = line_end(text, start);
            if end >= text.len() {
                off = text.len();
                break;
            }
            start = end + 1;
            if blank_line(text, start) {
                off = start;
                break;
            }
        }
    }
    off
}

/// `{`: the previous blank line, or the start of the buffer.
pub fn paragraph_back(text: &str, mut off: usize, count: u32) -> usize {
    for _ in 0..count.max(1) {
        let mut start = line_start(text, off);
        loop {
            if start == 0 {
                off = 0;
                break;
            }
            start = line_start(text, start - 1);
            if blank_line(text, start) {
                off = start;
                break;
            }
        }
    }
    off
}

// --- f F t T and % -----------------------------------------------------------------------------

/// `f`/`t` and their backward twins, on one line only — vim never crosses a newline for these.
/// `till` backs the answer off by one, which is the whole difference between `f` and `t`.
pub fn find_char(
    text: &str,
    off: usize,
    target: char,
    forward: bool,
    till: bool,
    count: u32,
) -> Option<usize> {
    let mut cur = off;
    for _ in 0..count.max(1) {
        if forward {
            let end = line_end(text, off);
            let mut probe = next(text, cur);
            // `t` repeated from where it last stopped would never move, so step past the target.
            if till && char_at(text, probe) == Some(target) {
                probe = next(text, probe);
            }
            loop {
                if probe >= end {
                    return None;
                }
                if char_at(text, probe) == Some(target) {
                    break;
                }
                probe = next(text, probe);
            }
            cur = probe;
        } else {
            let start = line_start(text, off);
            let mut probe = cur;
            if till && before(text, probe).is_some_and(|c| c == target) {
                probe = prev(text, probe);
            }
            loop {
                if probe <= start {
                    return None;
                }
                probe = prev(text, probe);
                if char_at(text, probe) == Some(target) {
                    break;
                }
            }
            cur = probe;
        }
    }
    Some(if till {
        if forward {
            prev(text, cur)
        } else {
            next(text, cur)
        }
    } else {
        cur
    })
}

const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

/// `%`: the bracket matching the one under the cursor, or the next one on the line.
pub fn matching(text: &str, off: usize) -> Option<usize> {
    let end = line_end(text, off);
    let mut here = off;
    let (open, close, forward) = loop {
        if here >= end {
            return None;
        }
        let c = char_at(text, here)?;
        if let Some(&(open, close)) = PAIRS.iter().find(|(open, _)| *open == c) {
            break (open, close, true);
        }
        if let Some(&(open, close)) = PAIRS.iter().find(|(_, close)| *close == c) {
            break (open, close, false);
        }
        here = next(text, here);
    };

    let mut depth = 0i32;
    let mut probe = here;
    loop {
        match char_at(text, probe) {
            Some(c) if c == open => depth += if forward { 1 } else { -1 },
            Some(c) if c == close => depth += if forward { -1 } else { 1 },
            None => return None,
            _ => {}
        }
        if depth == 0 {
            return Some(probe);
        }
        if forward {
            probe = next(text, probe);
            if probe >= text.len() {
                return None;
            }
        } else {
            if probe == 0 {
                return None;
            }
            probe = prev(text, probe);
        }
    }
}
