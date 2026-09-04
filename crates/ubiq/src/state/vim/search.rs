//! `/`, `?`, `n`, `N`, `*` and `#`.
//!
//! ponytail: the pattern is a plain substring, not a regex. It covers what a `/` is reached for
//! most of the time without pulling a regex engine through a module that is otherwise pure string
//! arithmetic. Upgrading means parsing the pattern here and nowhere else.

use super::motion::{Class, char_at, class, next, prev};

/// The `/` line as it is being typed, or the pattern the last one left behind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchLine {
    pub forward: bool,
    pub pattern: String,
    /// Whether a match has to be a whole word. `*` and `#` take the word under the cursor and mean
    /// that word, not any identifier it happens to be a piece of — `*` on `foo` must not stop on
    /// `foobar`. A pattern typed at `/` means exactly what was typed, so this is false there.
    pub whole_word: bool,
}

/// The next match after `off`, wrapping around the end of the buffer the way vim does.
pub fn next_match(text: &str, off: usize, line: &SearchLine, forward: bool) -> Option<usize> {
    if line.pattern.is_empty() {
        return None;
    }
    let hit = |at: usize| !line.whole_word || whole_word_at(text, at, line.pattern.len());

    if forward {
        let from = next(text, off);
        find_from(text, from, &line.pattern, hit).or_else(|| find_from(text, 0, &line.pattern, hit))
    } else {
        rfind_before(text, off, &line.pattern, hit)
            .or_else(|| rfind_before(text, text.len(), &line.pattern, hit))
    }
}

fn find_from(text: &str, from: usize, pattern: &str, hit: impl Fn(usize) -> bool) -> Option<usize> {
    let mut at = from;
    while let Some(i) = text.get(at..)?.find(pattern) {
        let found = at + i;
        if hit(found) {
            return Some(found);
        }
        at = next(text, found);
    }
    None
}

fn rfind_before(
    text: &str,
    before: usize,
    pattern: &str,
    hit: impl Fn(usize) -> bool,
) -> Option<usize> {
    let mut end = before;
    while let Some(found) = text.get(..end)?.rfind(pattern) {
        if hit(found) {
            return Some(found);
        }
        end = found;
    }
    None
}

/// Whether the match at `at` is bounded by something that is not a word character on both sides.
fn whole_word_at(text: &str, at: usize, len: usize) -> bool {
    let before = if at == 0 {
        None
    } else {
        char_at(text, prev(text, at))
    };
    let after = char_at(text, at + len);
    let free = |c: Option<char>| c.is_none_or(|c| class(c) != Class::Word);
    free(before) && free(after)
}
