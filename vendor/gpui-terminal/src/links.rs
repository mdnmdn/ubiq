//! URL detection for terminal output that is not annotated with OSC 8.
//!
//! Matches `http://` and `https://` URLs. File paths are left to the harness.

/// A URL found on one terminal line, in cell columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundUrl {
    /// Inclusive start column.
    pub start: usize,
    /// Exclusive end column.
    pub end: usize,
    /// The matched URL, trailing sentence punctuation stripped.
    pub uri: String,
}

/// Scan a line of terminal text for `http://` and `https://` URLs.
///
/// Columns are character indices, which matches the grid for ASCII URLs.
pub fn urls_in_line(line: &str) -> Vec<FoundUrl> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if let Some(scheme_len) = scheme_at(&chars, i) {
            let start = i;
            i += scheme_len;
            while i < chars.len() && is_url_body(chars[i]) {
                i += 1;
            }
            let mut end = i;
            while end > start + scheme_len && is_trailing_punct(chars[end - 1]) {
                end -= 1;
            }
            if end > start + scheme_len {
                let uri: String = chars[start..end].iter().collect();
                out.push(FoundUrl { start, end, uri });
            }
            continue;
        }
        i += 1;
    }
    out
}

/// The URI under `col`, if this line has one covering that column.
pub fn url_at(line: &str, col: usize) -> Option<String> {
    urls_in_line(line)
        .into_iter()
        .find(|found| col >= found.start && col < found.end)
        .map(|found| found.uri)
}

fn scheme_at(chars: &[char], i: usize) -> Option<usize> {
    const HTTP: &[char] = &['h', 't', 't', 'p', ':', '/', '/'];
    const HTTPS: &[char] = &['h', 't', 't', 'p', 's', ':', '/', '/'];
    if chars[i..].starts_with(HTTPS) {
        Some(HTTPS.len())
    } else if chars[i..].starts_with(HTTP) {
        Some(HTTP.len())
    } else {
        None
    }
}

fn is_url_body(c: char) -> bool {
    !c.is_whitespace() && !matches!(c, '<' | '>' | '"' | '\'' | ')' | ']')
}

fn is_trailing_punct(c: char) -> bool {
    matches!(c, '.' | ',' | ';' | ':' | '!' | '?')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_https_url() {
        let found = urls_in_line("see https://example.com/a for details");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].uri, "https://example.com/a");
        assert_eq!(found[0].start, 4);
        assert_eq!(found[0].end, 25);
    }

    #[test]
    fn finds_http_url() {
        let found = urls_in_line("http://localhost:8080");
        assert_eq!(found[0].uri, "http://localhost:8080");
    }

    #[test]
    fn strips_trailing_punctuation() {
        let found = urls_in_line("https://example.com.");
        assert_eq!(found[0].uri, "https://example.com");
    }

    #[test]
    fn ignores_file_paths_and_bare_words() {
        assert!(urls_in_line("/Users/mdn/src/main.rs").is_empty());
        assert!(urls_in_line("file:///tmp/x").is_empty());
        assert!(urls_in_line("example.com").is_empty());
    }

    #[test]
    fn url_at_column() {
        let line = "go https://x.test now";
        assert_eq!(url_at(line, 3), Some("https://x.test".into()));
        assert_eq!(url_at(line, 0), None);
        assert_eq!(url_at(line, 18), None);
    }
}
