//! GFM bare-autolink linkification, as a post-pass over the inline span lists.
//!
//! `pulldown-cmark` 0.13 does not linkify bare URLs and offers no option flag for it: a
//! `https://example.com` written without angle brackets stays one [`Inline::Text`]. The documents
//! this viewer is for are full of bare URLs, so the scan lives here instead.
//!
//! The pass runs once per [`crate::parse`], after the block table is built, and it only ever
//! *splits* a text span: the leading text, an `Enter(Link)`, the URL text, an `Exit(Link)`, the
//! trailing text. Every piece keeps the source bytes it came from, because a span's range is how
//! every consumer maps back to the source.
//!
//! This follows the GFM autolink extension closely enough to be useful, not exhaustively — see the
//! rules on [`find_autolinks`].

use std::ops::Range;

use crate::{Block, BlockKind, Inline, Mark};

/// Linkify a block table in place, recursing through every container kind.
///
/// Containers carry their inlines on the nested structure rather than on `Block::spans`, so list
/// items, table cells, quotes and footnote definitions all have to be walked; doing it uniformly is
/// the only way a bare URL behaves the same wherever it was written.
pub(crate) fn linkify_blocks(blocks: &mut [Block]) {
    for block in blocks.iter_mut() {
        linkify_spans(&mut block.spans);
        match &mut block.kind {
            BlockKind::Quote { blocks, .. } | BlockKind::FootnoteDefinition { blocks, .. } => {
                linkify_blocks(blocks)
            }
            BlockKind::List { items, .. } => {
                for item in items {
                    linkify_blocks(&mut item.blocks);
                }
            }
            BlockKind::Table { head, rows, .. } => {
                for cell in head.iter_mut().chain(rows.iter_mut().flatten()) {
                    linkify_spans(&mut cell.spans);
                }
            }
            _ => {}
        }
    }
}

/// Split the bare URLs in a span list into links, in place.
///
/// A free function over the span list so it is testable without a document.
///
/// The scan runs over *runs* of adjacent text spans, not over one span at a time, because
/// `pulldown-cmark` cuts a text event at every delimiter run it considered for emphasis:
/// `…/Rust_(programming_language)` arrives as three `Text` events and no single one of them holds
/// the whole URL. A run is a maximal sequence of text spans whose ranges are contiguous, so
/// concatenating them is byte-identical to the source they cover; a run that yields a match is
/// re-emitted from the match boundaries, and one that does not is left exactly as it was.
///
/// Three things the pass will not touch:
///
/// - text already inside a link or an image — GFM forbids nested links, and the mark stack is what
///   says whether we are inside one;
/// - [`Inline::Code`] and [`Inline::Html`], which are literal by definition, and which also break
///   a run;
/// - a text span whose byte length differs from its range's. `Inline::Text` is already unescaped,
///   so `&amp;` or `\_` inside it shifts every offset after it and there is no way to map a match
///   back to exact source bytes. Exactness wins over coverage: that span is skipped.
pub fn linkify_spans(spans: &mut Vec<(Range<usize>, Inline)>) {
    let worth_scanning = spans
        .iter()
        .any(|(_, i)| matches!(i, Inline::Text(t) if may_hold_link(t)));
    if !worth_scanning {
        return;
    }

    let src = std::mem::take(spans);
    let mut out: Vec<(Range<usize>, Inline)> = Vec::with_capacity(src.len());
    let mut link_depth = 0usize;
    let mut i = 0usize;

    while i < src.len() {
        match &src[i].1 {
            Inline::Enter(Mark::Link { .. } | Mark::Image { .. }) => link_depth += 1,
            Inline::Exit(Mark::Link { .. } | Mark::Image { .. }) => {
                link_depth = link_depth.saturating_sub(1)
            }
            _ => {}
        }
        if link_depth > 0 || !is_mappable_text(&src[i]) {
            out.push(src[i].clone());
            i += 1;
            continue;
        }

        let start = i;
        let mut j = i + 1;
        while j < src.len() && is_mappable_text(&src[j]) && src[j].0.start == src[j - 1].0.end {
            j += 1;
        }
        i = j;

        let run_start = src[start].0.start;
        let mut run = String::new();
        for (_, inline) in &src[start..j] {
            if let Inline::Text(t) = inline {
                run.push_str(t);
            }
        }

        let found = find_autolinks(&run);
        if found.is_empty() {
            out.extend_from_slice(&src[start..j]);
            continue;
        }

        let mut cut = 0usize;
        for (m, dest) in found {
            if m.start > cut {
                out.push((
                    run_start + cut..run_start + m.start,
                    Inline::Text(run[cut..m.start].to_string()),
                ));
            }
            let r = run_start + m.start..run_start + m.end;
            let mark = Mark::Link {
                dest,
                title: String::new(),
            };
            out.push((r.clone(), Inline::Enter(mark.clone())));
            out.push((r.clone(), Inline::Text(run[m.clone()].to_string())));
            out.push((r, Inline::Exit(mark)));
            cut = m.end;
        }
        if cut < run.len() {
            out.push((
                run_start + cut..run_start + run.len(),
                Inline::Text(run[cut..].to_string()),
            ));
        }
    }

    *spans = out;
}

/// A text span whose bytes are exactly `source[range]` — the only kind a match can be mapped back
/// through.
fn is_mappable_text(span: &(Range<usize>, Inline)) -> bool {
    matches!(&span.1, Inline::Text(t) if t.len() == span.0.len())
}

/// The cheap pre-filter: no candidate prefix, no scan.
fn may_hold_link(text: &str) -> bool {
    text.contains("http") || text.contains("www.") || text.contains('@')
}

/// Every bare autolink in a string, as `(byte range in the string, destination)`, in order.
///
/// The rules implemented, from the GFM autolink extension:
///
/// - schemes `http://` and `https://`, plus `www.`, which gets `http://` prefixed in the
///   destination while the visible text stays `www.…`;
/// - a match starts at a word boundary — the start of the string, whitespace, or one of `*_~(`;
/// - trailing `?`, `!`, `.`, `,`, `:`, `*`, `_`, `~`, `'`, `"` are not part of the URL;
/// - a trailing `)` is part of the URL only if the parentheses balance inside the match, so
///   `(see https://x.com/a)` links `https://x.com/a` while `https://x.com/a_(b)` keeps the `(b)`;
/// - a trailing `;` that closes an HTML entity (`&amp;`) is excluded, entity and all;
/// - a bare email address becomes a `mailto:` link.
///
/// The domain must have at least two dot-separated segments, and the last two must not contain an
/// underscore — that is GFM's rule, and it is what keeps `a_b@c` and `www.x` from linking.
pub fn find_autolinks(text: &str) -> Vec<(Range<usize>, String)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if !is_start_boundary(bytes, i) {
            i += 1;
            continue;
        }
        if let Some((end, dest)) = match_scheme(text, i).or_else(|| match_email(text, i)) {
            out.push((i..end, dest));
            i = end;
            continue;
        }
        i += 1;
    }
    out
}

/// GFM's own boundary: the start of the run, whitespace, or one of `*_~(`.
fn is_start_boundary(bytes: &[u8], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    let prev = bytes[i - 1];
    prev.is_ascii_whitespace() || matches!(prev, b'*' | b'_' | b'~' | b'(')
}

/// `http://`, `https://` or `www.` at `i`.
fn match_scheme(text: &str, i: usize) -> Option<(usize, String)> {
    let rest = text.get(i..)?;
    // `www.` is the host's first label, so it stays in the visible text; only the destination gains
    // a scheme.
    let (scheme_len, prefix) = ["https://", "http://"]
        .iter()
        .find_map(|s| starts_with_ci(rest, s).map(|n| (n, "")))
        .or_else(|| starts_with_ci(rest, "www.").map(|n| (n, "http://")))?;

    let bytes = text.as_bytes();
    let domain_start = i + scheme_len;
    let mut j = domain_start;
    while j < bytes.len() && is_domain_byte(bytes[j]) {
        j += 1;
    }
    let domain_end = j;
    // Then GFM's "zero or more non-space, non-`<` characters" — the path, query and fragment.
    while j < bytes.len() && !bytes[j].is_ascii_whitespace() && bytes[j] != b'<' {
        j += 1;
    }

    let end = strip_trailing(text, i, j);
    if end <= domain_start {
        return None;
    }
    if !valid_domain(&text[domain_start..domain_end.min(end)]) {
        return None;
    }
    Some((end, format!("{prefix}{}", &text[i..end])))
}

/// A bare email address whose local part starts at `i`.
fn match_email(text: &str, i: usize) -> Option<(usize, String)> {
    let bytes = text.as_bytes();
    let mut j = i;
    while j < bytes.len() && is_email_local_byte(bytes[j]) {
        j += 1;
    }
    if j == i || bytes.get(j) != Some(&b'@') {
        return None;
    }
    let domain_start = j + 1;
    let mut j = domain_start;
    while j < bytes.len() && is_domain_byte(bytes[j]) {
        j += 1;
    }
    // A trailing period is punctuation; a trailing hyphen or underscore invalidates the whole
    // match rather than being trimmed, which is GFM's rule.
    let mut end = j;
    while end > domain_start && bytes[end - 1] == b'.' {
        end -= 1;
    }
    if end > domain_start && matches!(bytes[end - 1], b'-' | b'_') {
        return None;
    }
    if !valid_domain(&text[domain_start..end]) {
        return None;
    }
    let matched = &text[i..end];
    Some((end, format!("mailto:{matched}")))
}

/// Walk the end of `text[start..end]` back over punctuation that is not part of the URL.
fn strip_trailing(text: &str, start: usize, end: usize) -> usize {
    let mut end = end;
    while end > start {
        let s = &text[start..end];
        match *s.as_bytes().last().expect("non-empty") {
            b'?' | b'!' | b'.' | b',' | b':' | b'*' | b'_' | b'~' | b'\'' | b'"' => end -= 1,
            b')' => {
                let opens = s.bytes().filter(|c| *c == b'(').count();
                let closes = s.bytes().filter(|c| *c == b')').count();
                // Balanced inside the match means the `)` belongs to the URL: `…/a_(b)` keeps it,
                // `(see …/a)` does not.
                if closes > opens {
                    end -= 1;
                } else {
                    break;
                }
            }
            b';' => match entity_start(s) {
                Some(amp) => end = start + amp,
                None => break,
            },
            _ => break,
        }
    }
    end
}

/// The offset of the `&` when `s` ends in something shaped like `&amp;`.
fn entity_start(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut k = bytes.len() - 1; // the `;`
    let letters_end = k;
    while k > 0 && bytes[k - 1].is_ascii_alphanumeric() {
        k -= 1;
    }
    (k < letters_end && k > 0 && bytes[k - 1] == b'&').then(|| k - 1)
}

fn is_domain_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')
}

fn is_email_local_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+')
}

/// At least two dot-separated, non-empty segments, no underscore in the last two.
fn valid_domain(domain: &str) -> bool {
    let domain = domain.trim_end_matches(['.', '-', '_']);
    let segments: Vec<&str> = domain.split('.').collect();
    if segments.len() < 2 || segments.iter().any(|s| s.is_empty()) {
        return false;
    }
    if segments[segments.len() - 2..]
        .iter()
        .any(|s| s.contains('_'))
    {
        return false;
    }
    segments
        .last()
        .and_then(|s| s.chars().next())
        .is_some_and(|c| c.is_ascii_alphanumeric())
}

/// The length of `needle` when `s` starts with it, case-insensitively — `HTTPS://` is a URL too.
fn starts_with_ci(s: &str, needle: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.len() < needle.len() {
        return None;
    }
    bytes[..needle.len()]
        .iter()
        .zip(needle.as_bytes())
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
        .then_some(needle.len())
}
