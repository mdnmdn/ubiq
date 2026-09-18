//! A reader for the `.bundle` container the help packer writes: the same `UBIQBND1` format
//! [`crate::web_assets::archive`] writes for a vendor bundle, read back here instead of in the UI
//! crate because unpacking a help bundle is this half's job — see the module doc on
//! [`super`]. Ported from `crates/ubiq/src/web_export/archive.rs`, whose reader this crate may not
//! depend on.
//!
//! Byte layout, exactly:
//!
//! ```text
//! [ deflate(entry 0 bytes) ][ deflate(entry 1) ] … [ index JSON ][ u64 LE index offset ][ magic b"UBIQBND1" ]
//! ```
//!
//! Each entry body is raw deflate, read back with [`flate2::read::DeflateDecoder`]. The index is
//! UTF-8 JSON: an array of `["<path>", offset, compressed_len, uncompressed_len]` tuples, offsets
//! counted from the start of the file. Paths are forward-slashed with no leading slash. The
//! footer is exactly the last 16 bytes of the file: a `u64` LE index offset, then the 8-byte
//! magic.
//!
//! Every structural problem — the file missing, too short for a footer, a bad magic, an index
//! that doesn't parse — is a `String` error, never a panic: a help bundle is a downgrade away from
//! a build with none, not something worth taking the process down over.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read as _, Seek, SeekFrom};
use std::path::Path;

use flate2::read::DeflateDecoder;

const MAGIC: &[u8; 8] = b"UBIQBND1";
const FOOTER_LEN: u64 = 16;

struct Entry {
    offset: u64,
    clen: u32,
    ulen: u32,
}

/// An open help bundle: the file handle plus its index, parsed once at [`Reader::open`].
pub struct Reader {
    file: File,
    index: HashMap<String, Entry>,
}

impl Reader {
    /// Reads the footer, checks the magic, and parses the index.
    pub fn open(path: &Path) -> Result<Reader, String> {
        let mut file = File::open(path).map_err(|err| format!("help bundle: {err}"))?;
        let len = file
            .metadata()
            .map_err(|err| format!("help bundle: {err}"))?
            .len();
        if len < FOOTER_LEN {
            return Err("help bundle: file shorter than its footer".to_string());
        }

        let mut footer = [0u8; FOOTER_LEN as usize];
        file.seek(SeekFrom::Start(len - FOOTER_LEN))
            .map_err(|err| format!("help bundle: {err}"))?;
        file.read_exact(&mut footer)
            .map_err(|err| format!("help bundle: {err}"))?;
        let (index_offset_bytes, magic) = footer.split_at(8);
        if magic != MAGIC {
            return Err("help bundle: bad magic".to_string());
        }
        let index_offset = u64::from_le_bytes(index_offset_bytes.try_into().unwrap());
        if index_offset > len - FOOTER_LEN {
            return Err("help bundle: index offset past the footer".to_string());
        }

        let index_len = (len - FOOTER_LEN - index_offset) as usize;
        file.seek(SeekFrom::Start(index_offset))
            .map_err(|err| format!("help bundle: {err}"))?;
        let mut index_bytes = vec![0u8; index_len];
        file.read_exact(&mut index_bytes)
            .map_err(|err| format!("help bundle: {err}"))?;
        let raw: Vec<(String, u64, u32, u32)> = serde_json::from_slice(&index_bytes)
            .map_err(|err| format!("help bundle: bad index: {err}"))?;
        let index = raw
            .into_iter()
            .map(|(path, offset, clen, ulen)| (path, Entry { offset, clen, ulen }))
            .collect();

        Ok(Reader { file, index })
    }

    /// Every path the index names, in no particular order — what extraction walks.
    pub fn entries(&self) -> impl Iterator<Item = &str> {
        self.index.keys().map(String::as_str)
    }

    /// Seeks to `path`'s entry, reads its `clen` compressed bytes, and inflates them. `None` for
    /// a name with no entry, and for an entry whose inflated length doesn't match its recorded
    /// `ulen` — a truncated or corrupt entry never comes back as a short body.
    pub fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        let entry = self.index.get(path)?;
        self.file.seek(SeekFrom::Start(entry.offset)).ok()?;
        let mut compressed = vec![0u8; entry.clen as usize];
        self.file.read_exact(&mut compressed).ok()?;
        let mut out = Vec::with_capacity(entry.ulen as usize);
        DeflateDecoder::new(&compressed[..])
            .read_to_end(&mut out)
            .ok()?;
        (out.len() == entry.ulen as usize).then_some(out)
    }
}

/// A minimal `UBIQBND1` writer for tests in this module and in [`super`]'s — the same layout the
/// real packer writes, so a fixture bundle exercises exactly the reader and the extractor it is
/// meant to.
#[cfg(test)]
pub(crate) mod fixture {
    use super::MAGIC;
    use flate2::Compression;
    use flate2::write::DeflateEncoder;
    use std::io::Write as _;
    use std::path::Path;

    fn deflate(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    pub(crate) fn write_bundle(path: &Path, entries: &[(&str, &[u8])]) {
        let mut bytes = Vec::new();
        let mut index = Vec::new();
        for (name, contents) in entries {
            let deflated = deflate(contents);
            let offset = bytes.len() as u64;
            bytes.extend_from_slice(&deflated);
            index.push((
                name.to_string(),
                offset,
                deflated.len() as u32,
                contents.len() as u32,
            ));
        }
        let index_offset = bytes.len() as u64;
        bytes.extend_from_slice(&serde_json::to_vec(&index).unwrap());
        bytes.extend_from_slice(&index_offset.to_le_bytes());
        bytes.extend_from_slice(MAGIC);
        std::fs::write(path, &bytes).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fixture::write_bundle;

    #[test]
    fn reads_two_entries_and_answers_none_for_a_third() {
        let a = b"hello world".to_vec();
        let b = b"a second entry, a bit longer than the first one".to_vec();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bundle.bin");
        write_bundle(&path, &[("a.txt", &a), ("dir/b.txt", &b)]);

        let mut reader = Reader::open(&path).unwrap();
        assert_eq!(reader.read("a.txt"), Some(a));
        assert_eq!(reader.read("dir/b.txt"), Some(b));
        assert_eq!(reader.read("missing.txt"), None, "an absent name is None");
    }

    #[test]
    fn a_short_file_is_a_string_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.bin");
        std::fs::write(&path, b"too short").unwrap();
        assert!(Reader::open(&path).is_err());
    }
}
