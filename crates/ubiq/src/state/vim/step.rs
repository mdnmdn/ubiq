//! The command set: one keystroke in, a list of effects out.
//!
//! Everything is a pure function of the state and the buffer, which is what lets
//! `crates/ubiq/tests/vim.rs` drive the whole of vim without a window.
//!
//! The shape is vim's own: a count, then an operator or a command, then — if it was an operator —
//! a motion or a text object. A key that completes nothing is kept in `pending` and the engine
//! waits; a key that fits nothing clears `pending` and does nothing at all.

use std::ops::Range;

use super::motion::{self, MotionKind, Move};
use super::search::{self, SearchLine};
use super::{CommandLine, Doc, Effect, Key, VimMode, VimState, object};

/// One indent step. ponytail: fixed, where the editor's own `TabSize` is per-language and not
/// reachable from a module that refuses to name the component.
const INDENT: &str = "    ";

/// How far `ctrl-d` and `ctrl-f` scroll. Vim uses half a screen and a screen; the engine has no
/// idea how tall the input is, so these are the sizes that feel like it in a normal-height pane.
const HALF_PAGE: isize = 15;
const PAGE: isize = 30;

pub fn step(st: &mut VimState, key: &Key, doc: Doc) -> Vec<Effect> {
    if st.typing.is_some() {
        return typing(st, key, &doc);
    }
    match st.mode {
        VimMode::Insert => insert(st, key, &doc),
        _ => normal(st, key, &doc),
    }
}

/// Insert mode sees nothing but Escape — `VimState::claims` lets every other key through to the
/// input untouched, which is what keeps typing, IME and the component's own shortcuts intact.
fn insert(st: &mut VimState, key: &Key, doc: &Doc) -> Vec<Effect> {
    if key.key != "escape" {
        return Vec::new();
    }
    st.mode = VimMode::Normal;
    st.clear_pending();
    // Vim steps the cursor back off the character it was about to type in front of, but never
    // past the start of the line.
    let cur = doc.sel.start;
    let back = motion::prev(doc.text, cur).max(motion::line_start(doc.text, cur));
    vec![Effect::Select(back..back)]
}

/// The command line. Every key is text for it until Enter runs it or Escape abandons it.
fn typing(st: &mut VimState, key: &Key, doc: &Doc) -> Vec<Effect> {
    let cur = doc.sel.start;
    match key.key.as_str() {
        "escape" => {
            st.typing = None;
            Vec::new()
        }
        "enter" => {
            let Some(line) = st.typing.take() else {
                return Vec::new();
            };
            match line.lead {
                ':' => ex(&line.text),
                lead => {
                    st.search = SearchLine {
                        forward: lead == '/',
                        pattern: line.text,
                        whole_word: false,
                    };
                    jump_to_match(st, doc, cur, true)
                }
            }
        }
        "backspace" => {
            // Rubbing out the lead itself closes the line, which is how vim's `:` behaves and
            // saves reaching for Escape.
            if let Some(line) = &mut st.typing
                && line.text.pop().is_none()
            {
                st.typing = None;
            }
            Vec::new()
        }
        _ => {
            let typed = if key.key == "space" {
                Some(' ')
            } else {
                key.ch()
            };
            if let (Some(c), Some(line)) = (typed, st.typing.as_mut()) {
                line.text.push(c);
            }
            Vec::new()
        }
    }
}

/// The ex commands, which is a deliberately short list: the two things a modal editor has to be
/// able to do without reaching for a modifier, and the way out of an edit that went wrong.
///
/// Anything else is ignored rather than guessed at — a `:` command that half-works is worse than
/// one that does nothing, because the user cannot tell which they got. `:set`, `:%s/`, ranges and
/// the rest are not here; `G100` in the backlog says so.
fn ex(line: &str) -> Vec<Effect> {
    match line.trim() {
        "w" => vec![Effect::Save],
        "q" => vec![Effect::Close { discard: false }],
        // `:qa` is this project's spelling of "close and throw the edit away". Vim spells that
        // `:q!` and means "quit every window" by `:qa`, so both are taken here: there is one
        // editor tab to close either way, and a vim user's fingers will type `:q!`.
        "qa" | "q!" | "qa!" => vec![Effect::Close { discard: true }],
        "wq" | "x" => vec![Effect::Save, Effect::Close { discard: false }],
        _ => Vec::new(),
    }
}

/// Normal and both visual modes. They differ in what a motion does with its answer, not in how the
/// keys are read, which is why they share a path.
fn normal(st: &mut VimState, key: &Key, doc: &Doc) -> Vec<Effect> {
    let cur = caret(st, doc);

    if key.key == "escape" {
        st.clear_pending();
        if st.mode.is_visual() {
            st.last_visual = Some(doc.sel.clone());
            st.mode = VimMode::Normal;
            return vec![Effect::Select(cur..cur)];
        }
        return Vec::new();
    }

    // Control keys never take part in a multi-key command, so they are answered before the pending
    // buffer is touched at all.
    if key.ctrl {
        st.clear_pending();
        return match key.key.as_str() {
            "r" => vec![Effect::Redo],
            "d" => scroll(st, doc, cur, HALF_PAGE),
            "u" => scroll(st, doc, cur, -HALF_PAGE),
            "f" => scroll(st, doc, cur, PAGE),
            "b" => scroll(st, doc, cur, -PAGE),
            _ => Vec::new(),
        };
    }

    let typed = if key.key == "space" {
        Some(' ')
    } else {
        key.ch()
    };

    // A digit is a count, except when it is the `0` that means "start of line" and except when a
    // pending `f` is waiting for a literal character to find.
    if let Some(c) = typed
        && c.is_ascii_digit()
        && !waits_for_char(&st.pending)
        && !(c == '0' && st.count.is_none())
    {
        let digit = c as u32 - '0' as u32;
        st.count = Some(
            st.count
                .unwrap_or(0)
                .saturating_mul(10)
                .saturating_add(digit),
        );
        return Vec::new();
    }

    let Some(c) = typed.or_else(|| named(&key.key)) else {
        st.clear_pending();
        return Vec::new();
    };
    st.pending.push(c);

    let count = st.count.unwrap_or(1).max(1);
    let pending = st.pending.clone();
    match resolve(st, doc, cur, &pending, count) {
        // Half a command: keep the keys and the count, and wait for the rest.
        Outcome::Wait => Vec::new(),
        Outcome::Done(effects) => {
            st.clear_pending();
            effects
        }
    }
}

/// Whether a keystroke finished a command or only got part-way through one.
enum Outcome {
    Wait,
    Done(Vec<Effect>),
}

impl From<Vec<Effect>> for Outcome {
    fn from(effects: Vec<Effect>) -> Self {
        Outcome::Done(effects)
    }
}

/// A key with a name rather than a character, as the command set spells it. Enter and the arrows
/// are motions in Normal mode; everything else is not a command and cancels whatever was pending.
fn named(key: &str) -> Option<char> {
    match key {
        "enter" => Some('\n'),
        "left" | "backspace" => Some('h'),
        "right" => Some('l'),
        "up" => Some('k'),
        "down" => Some('j'),
        "home" => Some('^'),
        "end" => Some('$'),
        _ => None,
    }
}

/// Whether the pending command is waiting for one literal character rather than a command key —
/// `f`, `F`, `t`, `T`, `r` and the register prefix `"`.
fn waits_for_char(pending: &str) -> bool {
    matches!(
        pending.chars().next_back(),
        Some('f' | 'F' | 't' | 'T' | 'r' | '"')
    )
}

/// Where the cursor is. A visual selection knows where its ends are but not which one is moving,
/// so that one is remembered instead of inferred.
fn caret(st: &VimState, doc: &Doc) -> usize {
    if st.mode.is_visual() {
        st.cursor
    } else {
        doc.sel.start
    }
}

/// The furthest right a Normal-mode caret may sit: on the last character of the line, never past
/// it. Visual and operator-pending both reach one further, which is why this is only applied to a
/// motion that is moving the caret and nothing else.
fn clamp(text: &str, off: usize) -> usize {
    let start = motion::line_start(text, off);
    let end = motion::line_end(text, off);
    if end > start {
        off.min(motion::prev(text, end))
    } else {
        start
    }
}

// --- the dispatch --------------------------------------------------------------------------------

fn resolve(st: &mut VimState, doc: &Doc, cur: usize, p: &str, n: u32) -> Outcome {
    let text = doc.text;

    // An operator waiting for what to operate on. In visual mode there is nothing to wait for —
    // the selection already says.
    if !st.mode.is_visual()
        && p != "gv"
        && let Some(op) = operator(p)
    {
        return operate(st, doc, cur, op, n);
    }

    if st.mode.is_visual()
        && let Some(outcome) = visual_command(st, doc, cur, p, n)
    {
        return outcome;
    }

    // A motion is the same key in every mode; what changes is whether the answer moves the caret
    // or extends the selection.
    match parse_motion(st, text, cur, n, p) {
        Parsed::Wait => return Outcome::Wait,
        Parsed::Motion(m) => return go(st, doc, m).into(),
        Parsed::None => {}
    }

    command(st, doc, cur, p, n)
}

/// The operator a pending command names, and the keys after it. `None` when the command does not
/// start with one, or when it is a `g` that has not yet said which.
fn operator(p: &str) -> Option<(&str, &str)> {
    for op in ["gU", "gu", "d", "c", "y", ">", "<", "="] {
        if let Some(rest) = p.strip_prefix(op) {
            return Some((op, rest));
        }
    }
    None
}

/// An operator plus whatever came after it: nothing yet, the operator doubled (`dd`), a text
/// object (`diw`) or a motion (`dw`).
fn operate(st: &mut VimState, doc: &Doc, cur: usize, (op, rest): (&str, &str), n: u32) -> Outcome {
    let text = doc.text;
    if rest.is_empty() {
        return Outcome::Wait; // waiting for a motion
    }

    // `dd`, `yy`, `>>` — the operator doubled means the lines it sits on.
    if rest == op {
        let range = line_range(text, cur, n);
        return apply(st, doc, op, range, true).into();
    }

    let mut chars = rest.chars();
    let first = chars.next().unwrap_or(' ');

    // `diw`, `ca(` — a text object needs two keys, so one alone waits.
    if object::is_prefix(first) {
        let Some(second) = chars.next() else {
            return Outcome::Wait;
        };
        let around = first == 'a';
        return match object::resolve(text, cur, around, second) {
            Some(range) => apply(st, doc, op, range, false).into(),
            None => Outcome::Done(Vec::new()),
        };
    }

    match parse_motion(st, text, cur, n, rest) {
        Parsed::Wait => Outcome::Wait,
        Parsed::None => Outcome::Done(Vec::new()),
        Parsed::Motion(m) => {
            let (range, linewise) = span(text, cur, m);
            apply(st, doc, op, range, linewise).into()
        }
    }
}

/// A visual-mode command that acts on the selection. `None` when the key is not one of them, so
/// the caller can try it as a motion instead.
fn visual_command(st: &mut VimState, doc: &Doc, cur: usize, p: &str, _n: u32) -> Option<Outcome> {
    let text = doc.text;
    let linewise = st.mode == VimMode::VisualLine;
    let range = selection(st, doc, cur);

    let op = match p {
        "d" | "x" => "d",
        "c" | "s" => "c",
        "y" => "y",
        ">" => ">",
        "<" => "<",
        "u" | "gu" => "gu",
        "U" | "gU" => "gU",
        "g" => return Some(Outcome::Wait), // waiting for the second key of `gu`/`gU`
        "o" => {
            // Swap which end of the selection moves.
            let other = if cur == range.start {
                range.end
            } else {
                range.start
            };
            st.anchor = cur;
            return Some(vec![Effect::Select(order(st.anchor, other))].into());
        }
        "~" => {
            let flipped: String = text[range.clone()].chars().flat_map(flip_case).collect();
            st.mode = VimMode::Normal;
            return Some(
                vec![
                    Effect::Replace(range.clone(), flipped),
                    Effect::Select(range.start..range.start),
                ]
                .into(),
            );
        }
        "p" | "P" => {
            let paste = st.register.clone();
            st.mode = VimMode::Normal;
            return Some(vec![Effect::Replace(range, paste)].into());
        }
        "J" => {
            st.mode = VimMode::Normal;
            return Some(join(text, range.start, 2).into());
        }
        _ => return None,
    };

    st.last_visual = Some(range.clone());
    st.mode = VimMode::Normal;
    Some(apply(st, doc, op, range, linewise).into())
}

/// Everything that is neither an operator nor a motion.
fn command(st: &mut VimState, doc: &Doc, cur: usize, p: &str, n: u32) -> Outcome {
    let text = doc.text;
    let effects = match p {
        // Waiting for the second key of a two-key command that is not an operator.
        "g" => return Outcome::Wait,

        "i" => enter_insert(st, cur),
        "I" => enter_insert(st, motion::first_non_blank(text, cur)),
        "a" => enter_insert(st, motion::next(text, cur)),
        "A" => enter_insert(st, motion::line_end(text, cur)),
        "o" => {
            let end = motion::line_end(text, cur);
            let indent = indent_of(text, cur);
            st.mode = VimMode::Insert;
            vec![Effect::Replace(end..end, format!("\n{indent}"))]
        }
        "O" => {
            let start = motion::line_start(text, cur);
            let indent = indent_of(text, cur);
            st.mode = VimMode::Insert;
            let at = start + indent.len();
            vec![
                Effect::Replace(start..start, format!("{indent}\n")),
                Effect::Select(at..at),
            ]
        }

        "x" => {
            let end = step_right(text, cur, n);
            take(st, text, cur..end, false);
            vec![Effect::Replace(cur..end, String::new())]
        }
        "X" => {
            let start = step_left(text, cur, n);
            take(st, text, start..cur, false);
            vec![Effect::Replace(start..cur, String::new())]
        }
        "D" => {
            let end = motion::line_end(text, cur);
            take(st, text, cur..end, false);
            vec![Effect::Replace(cur..end, String::new())]
        }
        "C" => {
            let end = motion::line_end(text, cur);
            take(st, text, cur..end, false);
            st.mode = VimMode::Insert;
            vec![Effect::Replace(cur..end, String::new())]
        }
        "S" => apply(st, doc, "c", line_range(text, cur, n), true),
        "Y" => apply(st, doc, "y", line_range(text, cur, n), true),
        "s" => {
            let end = step_right(text, cur, n);
            take(st, text, cur..end, false);
            st.mode = VimMode::Insert;
            vec![Effect::Replace(cur..end, String::new())]
        }
        "J" => join(text, cur, n.max(2)),
        "~" => {
            let end = motion::next(text, cur);
            let flipped: String = text[cur..end].chars().flat_map(flip_case).collect();
            vec![Effect::Replace(cur..end, flipped), Effect::Select(end..end)]
        }

        "p" | "P" => paste(st, text, cur, p == "p", n),
        "u" => vec![Effect::Undo],

        "v" => {
            st.mode = VimMode::Visual;
            st.anchor = cur;
            st.cursor = cur;
            vec![Effect::Select(cur..motion::next(text, cur))]
        }
        "V" => {
            st.mode = VimMode::VisualLine;
            st.anchor = cur;
            st.cursor = cur;
            vec![Effect::Select(line_range(text, cur, 1))]
        }
        "gv" => match st.last_visual.clone() {
            Some(range) => {
                st.mode = VimMode::Visual;
                st.anchor = range.start;
                st.cursor = range.end;
                vec![Effect::Select(range)]
            }
            None => Vec::new(),
        },

        "/" | "?" | ":" => {
            st.typing = Some(CommandLine {
                lead: p.chars().next().unwrap_or(':'),
                text: String::new(),
            });
            Vec::new()
        }
        "n" => jump_to_match(st, doc, cur, true),
        "N" => jump_to_match(st, doc, cur, false),
        "*" | "#" => match motion::word_at(text, cur) {
            Some(word) => {
                st.search = SearchLine {
                    forward: p == "*",
                    pattern: text[word.clone()].to_string(),
                    whole_word: true,
                };
                // From the word's own start, so the search lands on the next occurrence rather
                // than on the word it was handed — which is what a cursor sitting on a blank in
                // front of the first one would otherwise get.
                jump_to_match(st, doc, word.start, true)
            }
            None => Vec::new(),
        },

        _ => Vec::new(),
    };
    Outcome::Done(effects)
}

// --- motions -------------------------------------------------------------------------------------

enum Parsed {
    /// The keys so far could still become a motion — `g`, or `f` without its target.
    Wait,
    Motion(Move),
    None,
}

fn parse_motion(st: &mut VimState, text: &str, cur: usize, n: u32, p: &str) -> Parsed {
    let mut chars = p.chars();
    let first = chars.next().unwrap_or(' ');
    let second = chars.next();

    // `f`, `F`, `t`, `T` and `r` all want one literal character next.
    if matches!(first, 'f' | 'F' | 't' | 'T') {
        let Some(target) = second else {
            return Parsed::Wait;
        };
        let forward = first == 'f' || first == 't';
        let till = first == 't' || first == 'T';
        st.last_find = Some((target, forward, till));
        return match motion::find_char(text, cur, target, forward, till, n) {
            Some(to) => Parsed::Motion(Move::inclusive(to)),
            None => Parsed::None,
        };
    }

    if first == 'g' {
        return match second {
            None => Parsed::Wait,
            Some('g') => Parsed::Motion(Move::linewise(if st.count.is_some() {
                motion::nth_line(text, n)
            } else {
                0
            })),
            Some(_) => Parsed::None,
        };
    }

    if second.is_some() {
        return Parsed::None;
    }

    let m = match first {
        'h' => Move::exclusive(step_left(text, cur, n)),
        'l' | ' ' => Move::exclusive(step_right(text, cur, n)),
        'j' | '\n' => vertical(st, text, cur, n as isize),
        'k' => vertical(st, text, cur, -(n as isize)),
        '0' => Move::exclusive(motion::line_start(text, cur)),
        '^' => Move::exclusive(motion::first_non_blank(text, cur)),
        '$' => Move::inclusive(motion::prev(text, motion::line_end(text, cur))),
        'w' | 'W' => Move::exclusive(motion::word_forward(text, cur, n)),
        'b' | 'B' => Move::exclusive(motion::word_back(text, cur, n)),
        'e' | 'E' => Move::inclusive(motion::word_end(text, cur, n)),
        '{' => Move::exclusive(motion::paragraph_back(text, cur, n)),
        '}' => Move::exclusive(motion::paragraph_forward(text, cur, n)),
        'G' => Move::linewise(if st.count.is_some() {
            motion::nth_line(text, n)
        } else {
            motion::last_line(text)
        }),
        '%' => match motion::matching(text, cur) {
            Some(to) => Move::inclusive(to),
            None => return Parsed::None,
        },
        ';' | ',' => {
            let Some((target, forward, till)) = st.last_find else {
                return Parsed::None;
            };
            let forward = if first == ';' { forward } else { !forward };
            match motion::find_char(text, cur, target, forward, till, n) {
                Some(to) => Move::inclusive(to),
                None => return Parsed::None,
            }
        }
        _ => return Parsed::None,
    };
    Parsed::Motion(m)
}

/// A vertical motion, keeping the preferred column across short lines.
fn vertical(st: &mut VimState, text: &str, cur: usize, delta: isize) -> Move {
    let col = st
        .preferred_col
        .unwrap_or_else(|| motion::column(text, cur));
    st.preferred_col = Some(col);
    Move::linewise(motion::vertical(text, cur, delta, col))
}

/// What a motion does when nothing is operating on it: move the caret, or drag the visual
/// selection's free end.
fn go(st: &mut VimState, doc: &Doc, m: Move) -> Vec<Effect> {
    let text = doc.text;
    // Only `j` and `k` keep a preferred column; every other motion sets a new one.
    if m.kind != MotionKind::Linewise {
        st.preferred_col = None;
    }
    if st.mode.is_visual() {
        st.cursor = m.to;
    }
    match st.mode {
        VimMode::VisualLine => {
            let range = order(st.anchor, m.to);
            let start = motion::line_start(text, range.start);
            let end = (motion::line_end(text, range.end) + 1).min(text.len());
            vec![Effect::Select(start..end)]
        }
        VimMode::Visual => {
            let range = order(st.anchor, m.to);
            // The visual selection always covers the character the cursor sits on.
            let end = if m.to >= st.anchor {
                motion::next(text, range.end)
            } else {
                range.end
            };
            vec![Effect::Select(range.start..end)]
        }
        _ => {
            let to = clamp(text, m.to);
            vec![Effect::Select(to..to)]
        }
    }
}

fn scroll(st: &mut VimState, doc: &Doc, cur: usize, lines: isize) -> Vec<Effect> {
    let m = vertical(st, doc.text, cur, lines);
    go(st, doc, m)
}

// --- operators -----------------------------------------------------------------------------------

/// The byte range an operator covers, given where the cursor is and where the motion landed.
fn span(text: &str, cur: usize, m: Move) -> (Range<usize>, bool) {
    let (a, b) = (cur.min(m.to), cur.max(m.to));
    match m.kind {
        MotionKind::Exclusive => (a..b, false),
        MotionKind::Inclusive => (a..motion::next(text, b), false),
        MotionKind::Linewise => (whole_lines(text, a, b), true),
    }
}

/// `count` whole lines from the one the cursor is on, newline included.
fn line_range(text: &str, cur: usize, count: u32) -> Range<usize> {
    let start = motion::line_start(text, cur);
    let mut end = start;
    for _ in 0..count.max(1) {
        end = motion::line_end(text, end);
        if end >= text.len() {
            break;
        }
        end += 1;
    }
    whole_lines(text, start, end.saturating_sub(1).max(start))
}

/// From the start of `a`'s line to just past the end of `b`'s, taking the newline with it. The
/// last line of a buffer has none, so it takes the one in front of it instead — otherwise `dd`
/// there leaves an empty line behind.
fn whole_lines(text: &str, a: usize, b: usize) -> Range<usize> {
    let start = motion::line_start(text, a);
    let end = motion::line_end(text, b);
    if end < text.len() {
        start..end + 1
    } else if start > 0 {
        start - 1..end
    } else {
        start..end
    }
}

fn apply(
    st: &mut VimState,
    doc: &Doc,
    op: &str,
    range: Range<usize>,
    linewise: bool,
) -> Vec<Effect> {
    let text = doc.text;
    match op {
        "d" => {
            let yank = take(st, text, range.clone(), linewise);
            let mut effects = vec![Effect::Replace(range.clone(), String::new())];
            effects.insert(0, Effect::Yank(yank));
            effects
        }
        "c" => {
            // `cc` clears the line but keeps it, and keeps its indentation — deleting the newline
            // would make it a join rather than a change.
            let range = if linewise {
                trim_newline(text, range)
            } else {
                range
            };
            let yank = take(st, text, range.clone(), false);
            st.mode = VimMode::Insert;
            vec![Effect::Yank(yank), Effect::Replace(range, String::new())]
        }
        "y" => {
            let yank = take(st, text, range.clone(), linewise);
            vec![Effect::Yank(yank), Effect::Select(range.start..range.start)]
        }
        ">" | "<" => {
            let lines = whole_lines(
                text,
                range.start,
                range.end.saturating_sub(1).max(range.start),
            );
            let block = &text[lines.clone()];
            let shifted: String = block
                .split_inclusive('\n')
                .map(|line| {
                    if op == ">" {
                        format!("{INDENT}{line}")
                    } else {
                        line.strip_prefix(INDENT)
                            .or_else(|| line.strip_prefix('\t'))
                            .unwrap_or_else(|| line.trim_start_matches(' '))
                            .to_string()
                    }
                })
                .collect();
            let at = lines.start;
            vec![Effect::Replace(lines, shifted), Effect::Select(at..at)]
        }
        "gU" | "gu" => {
            let cased: String = if op == "gU" {
                text[range.clone()].to_uppercase()
            } else {
                text[range.clone()].to_lowercase()
            };
            let at = range.start;
            vec![Effect::Replace(range, cased), Effect::Select(at..at)]
        }
        // `=` is vim's reindent. There is no formatter behind it here, so it moves the caret and
        // changes nothing rather than pretending.
        _ => vec![Effect::Select(range.start..range.start)],
    }
}

/// Put a range into the register, and hand it back for the clipboard.
fn take(st: &mut VimState, text: &str, range: Range<usize>, linewise: bool) -> String {
    let taken = text[range].to_string();
    st.register = taken.clone();
    st.register_linewise = linewise;
    taken
}

fn trim_newline(text: &str, range: Range<usize>) -> Range<usize> {
    let end = if text[range.clone()].ends_with('\n') {
        motion::prev(text, range.end)
    } else {
        range.end
    };
    let start = motion::first_non_blank(text, range.start);
    start.min(end)..end
}

fn paste(st: &VimState, text: &str, cur: usize, after: bool, n: u32) -> Vec<Effect> {
    if st.register.is_empty() {
        return Vec::new();
    }
    let body = st.register.repeat(n.max(1) as usize);
    if st.register_linewise {
        let at = if after {
            (motion::line_end(text, cur) + 1).min(text.len())
        } else {
            motion::line_start(text, cur)
        };
        let body = if body.ends_with('\n') {
            body
        } else {
            format!("{body}\n")
        };
        let land = at;
        return vec![Effect::Replace(at..at, body), Effect::Select(land..land)];
    }
    let at = if after { motion::next(text, cur) } else { cur };
    vec![Effect::Replace(at..at, body)]
}

/// `J`: pull the next `count - 1` lines onto this one, with one space between and the leading
/// blanks of each dropped.
fn join(text: &str, cur: usize, count: u32) -> Vec<Effect> {
    let mut effects = Vec::new();
    let end = motion::line_end(text, cur);
    if end >= text.len() {
        return effects;
    }
    let next_line = motion::first_non_blank(text, end + 1);
    // Only the first join is computed against the buffer as it stands; a count beyond two would
    // need the text after the edit, which this module never sees.
    let _ = count;
    effects.push(Effect::Replace(end..next_line, " ".to_string()));
    effects.push(Effect::Select(end..end));
    effects
}

// --- small helpers ---------------------------------------------------------------------------------

fn enter_insert(st: &mut VimState, at: usize) -> Vec<Effect> {
    st.mode = VimMode::Insert;
    vec![Effect::Select(at..at)]
}

fn selection(st: &VimState, doc: &Doc, cur: usize) -> Range<usize> {
    if doc.sel.start == doc.sel.end {
        order(st.anchor, cur)
    } else {
        doc.sel.clone()
    }
}

fn order(a: usize, b: usize) -> Range<usize> {
    if a <= b { a..b } else { b..a }
}

/// `h` and friends never cross onto the previous line, the way vim's `nowhichwrap` default does.
fn step_left(text: &str, cur: usize, n: u32) -> usize {
    let floor = motion::line_start(text, cur);
    let mut off = cur;
    for _ in 0..n.max(1) {
        if off <= floor {
            break;
        }
        off = motion::prev(text, off);
    }
    off
}

fn step_right(text: &str, cur: usize, n: u32) -> usize {
    let ceiling = motion::line_end(text, cur);
    let mut off = cur;
    for _ in 0..n.max(1) {
        if off >= ceiling {
            break;
        }
        off = motion::next(text, off);
    }
    off
}

/// The leading blanks of the cursor's line, so `o` and `O` open where the eye expects.
fn indent_of(text: &str, cur: usize) -> String {
    let start = motion::line_start(text, cur);
    text[start..motion::first_non_blank(text, cur)].to_string()
}

/// `~`: upper becomes lower and everything else becomes upper. A `Vec` because a case change is
/// not always one character long.
fn flip_case(c: char) -> std::vec::IntoIter<char> {
    let flipped: Vec<char> = if c.is_uppercase() {
        c.to_lowercase().collect()
    } else {
        c.to_uppercase().collect()
    };
    flipped.into_iter()
}

fn jump_to_match(st: &VimState, doc: &Doc, cur: usize, same_way: bool) -> Vec<Effect> {
    let forward = st.search.forward == same_way;
    match search::next_match(doc.text, cur, &st.search, forward) {
        Some(to) => vec![Effect::Select(to..to)],
        None => Vec::new(),
    }
}
