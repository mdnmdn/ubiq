//! Picking a URL out of a pane's own output, without touching it.
//!
//! A login prints an OAuth or device-code URL for the user to open, and a terminal is a poor
//! place to click one. [`LinkScanner`] reads the bytes on their way past — after they are already
//! queued for [`crate::pty::forward_output`]'s ordinary `Message::TerminalOutput` — and reports
//! each distinct URL once, so the host can also send a `Message::HarnessLoginLink`. It changes
//! nothing and forwards nothing: the pane still receives every byte, so what the user reads is
//! the harness's own output either way.

/// How much unmatched tail to keep between reads, so a URL split across two chunks is not
/// missed. A URL longer than this is truncated rather than the buffer growing — the whole memory
/// story of this type.
const TAIL_CAP: usize = 4 * 1024;

/// How many distinct URLs to remember. A chatty harness (or one replaying its own banner) stops
/// growing this past the cap rather than being tracked forever.
const SEEN_CAP: usize = 16;

/// Picks the URLs out of a byte stream without interpreting it.
///
/// Not a URL parser and not a VT parser — see the `ponytail:` notes below for exactly what that
/// leaves on the table.
pub struct LinkScanner {
    tail: Vec<u8>,
    seen: Vec<String>,
}

impl LinkScanner {
    pub fn new() -> Self {
        Self {
            tail: Vec::new(),
            seen: Vec::new(),
        }
    }

    /// Feed the next chunk; returns the URLs seen for the first time in it.
    ///
    /// The tail is never trimmed down to "what's left to match" — only capped by length. A
    /// straddling URL is caught because the bytes before and after the split both sit in the
    /// same bounded window at once; a URL already reported sitting in that window again is
    /// simply skipped by the `seen` check below, so re-scanning it costs nothing but a compare.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        self.tail.extend_from_slice(bytes);
        if self.tail.len() > TAIL_CAP {
            let drop = self.tail.len() - TAIL_CAP;
            self.tail.drain(..drop);
        }

        // ponytail: crude escape strip, not a VT parser — a URL split by a cursor move or other
        // non-CSI/OSC escape mid-run is missed. Good enough to keep colour codes out of a URL.
        let text = String::from_utf8_lossy(&self.tail);
        let stripped = strip_escapes(&text);

        let mut found = Vec::new();
        for url in find_urls(&stripped) {
            if self.seen.len() >= SEEN_CAP {
                break;
            }
            if !self.seen.iter().any(|s| s == &url) {
                self.seen.push(url.clone());
                found.push(url);
            }
        }

        found
    }
}

impl Default for LinkScanner {
    fn default() -> Self {
        Self::new()
    }
}

/// Drop `ESC [ ... <final byte>` (CSI) and `ESC ] ... (BEL | ESC \)` (OSC) runs.
///
/// ponytail: crude strip, not a VT parser — an escape this does not recognise (a lone ESC not
/// followed by `[` or `]`) is left in place rather than interpreted.
fn strip_escapes(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // CSI: ESC [ ... final byte in 0x40..=0x7e
            let mut j = i + 2;
            while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                j += 1;
            }
            i = (j + 1).min(bytes.len());
        } else if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b']' {
            // OSC: ESC ] ... BEL, or ESC ] ... ESC \
            let mut j = i + 2;
            while j < bytes.len() && bytes[j] != 0x07 {
                if bytes[j] == 0x1b && j + 1 < bytes.len() && bytes[j + 1] == b'\\' {
                    j += 2;
                    break;
                }
                j += 1;
            }
            i = (j + 1).min(bytes.len());
        } else {
            // Safe: `text` is a `&str`, so indexing forward one byte at a time and pushing via
            // `char_indices` semantics would be nicer, but a plain byte copy is fine here because
            // we only ever skip whole escape runs above and otherwise advance one UTF-8-safe
            // character at a time below.
            let ch_len = text[i..].chars().next().map(char::len_utf8).unwrap_or(1);
            out.push_str(&text[i..(i + ch_len).min(bytes.len())]);
            i += ch_len;
        }
    }
    out
}

/// Find complete `http://`/`https://` URLs in `text` — one whose terminator (whitespace, a
/// control byte, or one of `"'<>`\`) was actually seen. A URL still running off the end of
/// `text` is not returned; it may still be growing, and [`LinkScanner::feed`] simply rescans it
/// once more bytes arrive.
fn find_urls(text: &str) -> Vec<String> {
    let mut complete = Vec::new();
    let mut search_from = 0;

    while let Some(rel) = find_scheme(&text[search_from..]) {
        let start = search_from + rel;
        let rest = &text[start..];
        let Some(end) =
            rest.find(|c: char| c.is_whitespace() || c.is_control() || "\"'<>`\\".contains(c))
        else {
            break; // No terminator seen yet — this may still grow with the next chunk.
        };

        let raw = &rest[..end];
        let trimmed = raw.trim_end_matches(['.', ',', ')', ']', ';', ':']);
        if !trimmed.is_empty() {
            complete.push(trimmed.to_string());
        }
        search_from = start + end;
    }

    complete
}

/// The byte offset of the next `http://` or `https://` in `text`, if any.
fn find_scheme(text: &str) -> Option<usize> {
    let https = text.find("https://");
    let http = text.find("http://");
    match (https, http) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_url_in_one_chunk() {
        let mut scanner = LinkScanner::new();
        let found = scanner.feed(b"open https://example.com/login?x=1 to continue\n");
        assert_eq!(found, vec!["https://example.com/login?x=1"]);
    }

    #[test]
    fn url_split_across_two_feeds() {
        let mut scanner = LinkScanner::new();
        assert!(scanner.feed(b"go to https://exam").is_empty());
        let found = scanner.feed(b"ple.com/path now\n");
        assert_eq!(found, vec!["https://example.com/path"]);
    }

    #[test]
    fn url_wrapped_in_ansi_colour() {
        let mut scanner = LinkScanner::new();
        let found = scanner.feed(b"\x1b[34mhttps://example.com/login\x1b[0m\n");
        assert_eq!(found, vec!["https://example.com/login"]);
    }

    #[test]
    fn trailing_punctuation_trimmed() {
        let mut scanner = LinkScanner::new();
        let found = scanner.feed(b"see (https://example.com/a).\n");
        assert_eq!(found, vec!["https://example.com/a"]);
    }

    #[test]
    fn same_url_fed_twice_emitted_once() {
        let mut scanner = LinkScanner::new();
        let first = scanner.feed(b"https://example.com/login\n");
        let second = scanner.feed(b"https://example.com/login\n");
        assert_eq!(first, vec!["https://example.com/login"]);
        assert!(second.is_empty());
    }

    #[test]
    fn tail_buffer_stays_bounded_with_no_url() {
        let mut scanner = LinkScanner::new();
        let chunk = vec![b'x'; 1024];
        for _ in 0..64 {
            let found = scanner.feed(&chunk);
            assert!(found.is_empty());
        }
        assert!(scanner.tail.len() <= TAIL_CAP);
    }

    #[test]
    fn non_utf8_bytes_do_not_panic() {
        let mut scanner = LinkScanner::new();
        let mut chunk = b"https://example.com/".to_vec();
        chunk.extend_from_slice(&[0xff, 0xfe, 0x80]);
        chunk.extend_from_slice(b" more text\n");
        let _ = scanner.feed(&chunk);
        // Reaching this line without panicking is the assertion.
    }

    #[test]
    fn seen_cap_stops_growing_after_many_distinct_urls() {
        let mut scanner = LinkScanner::new();
        for i in 0..(SEEN_CAP + 8) {
            scanner.feed(format!("https://example.com/{i}\n").as_bytes());
        }
        assert!(scanner.seen.len() <= SEEN_CAP);
    }
}
