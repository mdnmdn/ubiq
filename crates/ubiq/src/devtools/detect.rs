//! What a pasted text most likely is.
//!
//! Every rule is a shape check followed, where the shape is ambiguous, by actually decoding — a
//! string is only called Base64 if it decodes to readable text, a JWT only if its header is JSON
//! naming an `alg`. The answer is a list in priority order, most specific first: a JWT is also
//! valid Base64url and three dotted segments, and only the first is worth suggesting.

use crate::devtools::{codec, hash, jwt, time, uuids};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Jwt,
    Uuid,
    /// Hex of a digest's length, with the algorithm that length most likely means.
    HexDigest(&'static str),
    UnixSeconds,
    UnixMillis,
    /// A date written out as text.
    Date,
    /// An `http(s)://` URL with `%XX` escapes in it.
    EncodedUrl,
    /// An `http(s)://` URL with nothing escaped.
    Url,
    /// Text carrying `%XX` escapes that decode to UTF-8.
    UrlEncoded,
    Json,
    Base64,
}

impl Kind {
    /// The noun phrase a suggestion reads with: "Clipboard looks like *a JWT*".
    pub fn label(self) -> String {
        match self {
            Kind::Jwt => "a JWT".to_string(),
            Kind::Uuid => "a UUID".to_string(),
            Kind::HexDigest(algorithm) => format!("a {algorithm} digest"),
            Kind::UnixSeconds => "a Unix timestamp (seconds)".to_string(),
            Kind::UnixMillis => "a Unix timestamp (milliseconds)".to_string(),
            Kind::Date => "a date".to_string(),
            Kind::EncodedUrl => "a URL with escapes".to_string(),
            Kind::Url => "a URL".to_string(),
            Kind::UrlEncoded => "URL-encoded text".to_string(),
            Kind::Json => "JSON".to_string(),
            Kind::Base64 => "Base64".to_string(),
        }
    }
}

/// Everything `text` could be, most specific first. Empty for plain prose.
pub fn detect(text: &str) -> Vec<Kind> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let single_token = !text.contains(char::is_whitespace);

    if single_token && jwt::looks_like(text) {
        // A JWT is also Base64url segments; nothing else is worth saying about it.
        return vec![Kind::Jwt];
    }
    if single_token && uuids::looks_like(text) {
        found.push(Kind::Uuid);
    }
    if text.bytes().all(|b| b.is_ascii_hexdigit())
        && let Some(algorithm) = hash::algorithm_for_hex_len(text.len())
        // Thirteen digits is a millisecond timestamp, not an accident of hex.
        && !text.bytes().all(|b| b.is_ascii_digit())
    {
        found.push(Kind::HexDigest(algorithm));
    }
    if let Some(kind) = timestamp(text) {
        found.push(kind);
    } else if looks_like_date(text) && time::parse_text(text, &chrono::Utc).is_some() {
        found.push(Kind::Date);
    }
    let lower = text.to_ascii_lowercase();
    let is_url = single_token && (lower.starts_with("http://") || lower.starts_with("https://"));
    let escaped = codec::has_percent_escape(text) && codec::url_decode(text, false).is_ok();
    if is_url {
        found.push(if escaped { Kind::EncodedUrl } else { Kind::Url });
    } else if escaped {
        found.push(Kind::UrlEncoded);
    }
    if (text.starts_with('{') || text.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(text).is_ok()
    {
        found.push(Kind::Json);
    }
    if found.is_empty() && looks_like_base64(text) {
        found.push(Kind::Base64);
    }
    found
}

/// Nine to ten digits is seconds between 1973 and 2286; twelve to thirteen is milliseconds over
/// the same span. Anything else made of digits is just a number.
fn timestamp(text: &str) -> Option<Kind> {
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match text.len() {
        9 | 10 => Some(Kind::UnixSeconds),
        12 | 13 => Some(Kind::UnixMillis),
        _ => None,
    }
}

/// A cheap gate before parsing: a date starts with a digit or a weekday and has a separator in
/// its first few characters.
fn looks_like_date(text: &str) -> bool {
    text.len() >= 8
        && text.len() <= 64
        && text
            .chars()
            .take(12)
            .any(|c| matches!(c, '-' | '/' | '.' | ','))
}

fn looks_like_base64(text: &str) -> bool {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() < 8 {
        return false;
    }
    let alphabet = compact
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'-' | b'_' | b'='));
    // A word of letters alone is far likelier to be a word.
    let mixed = compact
        .bytes()
        .any(|b| b.is_ascii_digit() || !b.is_ascii_alphabetic())
        || compact.bytes().any(|b| b.is_ascii_uppercase())
            && compact.bytes().any(|b| b.is_ascii_lowercase());
    alphabet
        && mixed
        && codec::b64_decode_text(&compact).is_ok_and(|decoded| codec::is_printable(&decoded))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devtools::codec::{B64Alphabet, b64_encode};

    #[test]
    fn jwt_wins_alone() {
        let token = format!(
            "{}.{}.sig",
            b64_encode(br#"{"alg":"HS256"}"#, B64Alphabet::UrlSafe),
            b64_encode(br#"{"sub":"1"}"#, B64Alphabet::UrlSafe)
        );
        assert_eq!(detect(&token), vec![Kind::Jwt]);
    }

    #[test]
    fn ids_digests_and_times() {
        assert_eq!(
            detect("67e55044-10b1-426f-9247-bb680e5fe0c8"),
            vec![Kind::Uuid]
        );
        assert_eq!(
            detect("900150983cd24fb0d6963f7d28e17f72"),
            vec![Kind::HexDigest("MD5")]
        );
        assert_eq!(detect(&"a".repeat(64)), vec![Kind::HexDigest("SHA-256")]);
        assert_eq!(detect("1700000000"), vec![Kind::UnixSeconds]);
        assert_eq!(detect("1700000000123"), vec![Kind::UnixMillis]);
        assert_eq!(detect("2023-11-14T22:13:20Z"), vec![Kind::Date]);
        assert_eq!(detect("Tue, 14 Nov 2023 22:13:20 +0000"), vec![Kind::Date]);
    }

    #[test]
    fn urls_json_and_base64() {
        assert_eq!(detect("https://example.com/a"), vec![Kind::Url]);
        assert_eq!(
            detect("https://example.com/?q=a%20b"),
            vec![Kind::EncodedUrl]
        );
        assert_eq!(detect("a%20b%2Fc"), vec![Kind::UrlEncoded]);
        assert_eq!(detect(r#"{"a": [1, 2]}"#), vec![Kind::Json]);
        assert_eq!(detect("aGVsbG8gd29ybGQ="), vec![Kind::Base64]);
    }

    #[test]
    fn prose_is_nothing() {
        assert!(detect("").is_empty());
        assert!(detect("hello there, how are you").is_empty());
        assert!(detect("password").is_empty());
        assert!(detect("12345").is_empty());
    }
}
