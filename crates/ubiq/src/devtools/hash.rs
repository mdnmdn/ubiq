//! Message digests of a text, every algorithm at once.

use md5::Md5;
use sha1::Sha1;
use sha2::{Digest as _, Sha256, Sha384, Sha512};

/// The algorithms the section shows, in the order it shows them.
pub const ALGORITHMS: [&str; 5] = ["MD5", "SHA-1", "SHA-256", "SHA-384", "SHA-512"];

/// Lowercase hex, two digits a byte.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Every digest of `input`, named, in [`ALGORITHMS`] order.
pub fn digests(input: &[u8]) -> Vec<(&'static str, String)> {
    vec![
        (ALGORITHMS[0], hex(&Md5::digest(input))),
        (ALGORITHMS[1], hex(&Sha1::digest(input))),
        (ALGORITHMS[2], hex(&Sha256::digest(input))),
        (ALGORITHMS[3], hex(&Sha384::digest(input))),
        (ALGORITHMS[4], hex(&Sha512::digest(input))),
    ]
}

/// The algorithm a hex digest of this many characters most likely came from.
pub fn algorithm_for_hex_len(len: usize) -> Option<&'static str> {
    match len {
        32 => Some(ALGORITHMS[0]),
        40 => Some(ALGORITHMS[1]),
        64 => Some(ALGORITHMS[2]),
        96 => Some(ALGORITHMS[3]),
        128 => Some(ALGORITHMS[4]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digests_of_abc() {
        let all = digests(b"abc");
        assert_eq!(all[0].1, "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(all[1].1, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            all[2].1,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(all[3].1.len(), 96);
        assert_eq!(all[4].1.len(), 128);
        for (name, value) in &all {
            assert_eq!(algorithm_for_hex_len(value.len()), Some(*name));
        }
    }

    #[test]
    fn hex_is_lowercase_and_padded() {
        assert_eq!(hex(&[0, 15, 255]), "000fff");
    }
}
