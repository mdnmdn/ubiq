//! The small decoders both formats need: base64, `data:` URIs, percent-encoding, raw deflate.

use std::io::Read;

/// Standard and URL-safe base64, whitespace tolerated. Thirty lines is cheaper than a dependency.
pub fn base64(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for byte in input.bytes() {
        let sextet = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(sextet);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// `data:image/png;base64,…` into its mime type and bytes. draw.io also writes
/// `data:image/png,…` with base64 and no `;base64` marker, so a payload that is not marked is
/// tried as base64 too. Anything else — a remote URL — is not a file the drawing carries.
pub fn data_uri(url: &str) -> Option<(String, Vec<u8>)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    let meta = meta.strip_suffix(";base64").unwrap_or(meta);
    let mime = meta.split(';').next().unwrap_or_default();
    Some((mime.to_string(), base64(payload)?))
}

/// `decodeURIComponent`, leniently: a malformed escape stays as written.
pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = input.get(i + 1..i + 3)
            && let Ok(value) = u8::from_str_radix(hex, 16)
        {
            out.push(value);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The most a compressed diagram may inflate to. A real page is kilobytes; anything past this
/// is a deflate bomb, and is refused rather than allocated.
pub const MAX_INFLATED: u64 = 32 * 1024 * 1024;

/// Raw deflate (no zlib header), as draw.io's `pako.deflateRaw` writes it. `None` when the
/// stream is corrupt or would inflate past [`MAX_INFLATED`].
pub fn inflate_raw(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::DeflateDecoder::new(bytes)
        .take(MAX_INFLATED + 1)
        .read_to_end(&mut out)
        .ok()?;
    (out.len() as u64 <= MAX_INFLATED).then_some(out)
}
