//! Base64 and percent-encoding, both directions.

use base64::Engine as _;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

/// Which Base64 alphabet an encode writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum B64Alphabet {
    /// `+` and `/`, padded with `=` — RFC 4648 §4.
    #[default]
    Standard,
    /// `-` and `_`, unpadded — RFC 4648 §5, what a JWT and most URLs carry.
    UrlSafe,
}

/// A decoder that takes either padding, so a pasted value never fails over a missing `=`.
const LENIENT: GeneralPurposeConfig = GeneralPurposeConfig::new()
    .with_decode_padding_mode(DecodePaddingMode::Indifferent)
    .with_decode_allow_trailing_bits(true);
const STANDARD_LENIENT: GeneralPurpose = GeneralPurpose::new(&alphabet::STANDARD, LENIENT);
const URL_LENIENT: GeneralPurpose = GeneralPurpose::new(&alphabet::URL_SAFE, LENIENT);

pub fn b64_encode(input: &[u8], alphabet: B64Alphabet) -> String {
    match alphabet {
        B64Alphabet::Standard => base64::engine::general_purpose::STANDARD.encode(input),
        B64Alphabet::UrlSafe => base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(input),
    }
}

/// Decode either alphabet, padded or not, ignoring whitespace — a value pasted out of a log or a
/// wrapped PEM body decodes as it is. The alphabet is read off the text: a `-` or `_` is URL-safe.
pub fn b64_decode(input: &str) -> Result<Vec<u8>, String> {
    let clean: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    let engine = if clean.contains(['-', '_']) {
        &URL_LENIENT
    } else {
        &STANDARD_LENIENT
    };
    engine.decode(clean.as_bytes()).map_err(|e| e.to_string())
}

/// [`b64_decode`] read as text. Bytes that are not UTF-8 are an error naming how many there were
/// and the first of them in hex, since there is nothing honest to draw them as.
pub fn b64_decode_text(input: &str) -> Result<String, String> {
    let bytes = b64_decode(input)?;
    String::from_utf8(bytes).map_err(|e| {
        let bytes = e.into_bytes();
        format!(
            "Decoded {} bytes that are not UTF-8 text: {}{}",
            bytes.len(),
            crate::devtools::hash::hex(&bytes[..bytes.len().min(32)]),
            if bytes.len() > 32 { "…" } else { "" }
        )
    })
}

/// Whether a decode of `text` is something a person would call text: UTF-8, and no control
/// character other than whitespace. What the detector asks before it calls a string Base64.
pub fn is_printable(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
}

/// RFC 3986's unreserved set is all a component keeps: everything else is escaped.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// A whole URL keeps its structure too — `encodeURI`'s reserved and sub-delimiter characters.
const FULL: &AsciiSet = &COMPONENT
    .remove(b';')
    .remove(b',')
    .remove(b'/')
    .remove(b'?')
    .remove(b':')
    .remove(b'@')
    .remove(b'&')
    .remove(b'=')
    .remove(b'+')
    .remove(b'$')
    .remove(b'#')
    .remove(b'!')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')')
    .remove(b'[')
    .remove(b']');

/// Percent-encode: `full` keeps a URL's delimiters, otherwise the text is one component.
pub fn url_encode(input: &str, full: bool) -> String {
    let set = if full { FULL } else { COMPONENT };
    utf8_percent_encode(input, set).to_string()
}

/// Percent-decode. `plus_as_space` reads `+` the way a form-encoded query string means it.
pub fn url_decode(input: &str, plus_as_space: bool) -> Result<String, String> {
    let owned;
    let source = if plus_as_space {
        owned = input.replace('+', " ");
        owned.as_str()
    } else {
        input
    };
    percent_decode_str(source)
        .decode_utf8()
        .map(|text| text.into_owned())
        .map_err(|_| "The escapes decode to bytes that are not UTF-8 text".to_string())
}

/// Whether `text` carries at least one `%XX` escape.
pub fn has_percent_escape(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes
        .windows(3)
        .any(|w| w[0] == b'%' && w[1].is_ascii_hexdigit() && w[2].is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_both_alphabets() {
        let text = "héllo?>>";
        let std = b64_encode(text.as_bytes(), B64Alphabet::Standard);
        let url = b64_encode(text.as_bytes(), B64Alphabet::UrlSafe);
        assert_eq!(std, "aMOpbGxvPz4+");
        assert_eq!(url, "aMOpbGxvPz4-");
        assert_eq!(b64_decode_text(&std).unwrap(), text);
        assert_eq!(b64_decode_text(&url).unwrap(), text);
    }

    #[test]
    fn base64_decode_is_lenient_about_padding_and_whitespace() {
        assert_eq!(b64_decode_text("aGk").unwrap(), "hi");
        assert_eq!(b64_decode_text("aGk=").unwrap(), "hi");
        assert_eq!(b64_decode_text(" aG\nk= ").unwrap(), "hi");
        assert!(b64_decode_text("!!!").is_err());
    }

    #[test]
    fn base64_names_binary_rather_than_drawing_it() {
        let err = b64_decode_text("/w==").unwrap_err();
        assert!(err.contains("1 bytes"), "{err}");
        assert!(err.contains("ff"), "{err}");
    }

    #[test]
    fn url_component_and_full() {
        assert_eq!(url_encode("a b&c/d", false), "a%20b%26c%2Fd");
        assert_eq!(
            url_encode("https://x.y/p q?a=1&b=é", true),
            "https://x.y/p%20q?a=1&b=%C3%A9"
        );
        assert_eq!(url_decode("a%20b%26c", false).unwrap(), "a b&c");
        assert_eq!(url_decode("a+b", false).unwrap(), "a+b");
        assert_eq!(url_decode("a+b", true).unwrap(), "a b");
        assert!(url_decode("%FF", false).is_err());
    }

    #[test]
    fn percent_escape_detection() {
        assert!(has_percent_escape("a%2Fb"));
        assert!(!has_percent_escape("100%"));
        assert!(!has_percent_escape("%zz"));
    }
}
