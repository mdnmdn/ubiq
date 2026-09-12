//! The vendor bundle archive: one compressed file on disk in place of a directory of hundreds of
//! loose files. The host writes it; this module only ever reads it, and never extracts an entry
//! to disk — [`Reader::read`] seeks to one entry, inflates it in memory, and hands back the bytes.
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

/// An open vendor archive: the file handle plus its index, parsed once at [`Reader::open`].
/// Deliberately not `Clone` — a route locks the `WebChannel` that owns one and reads a single
/// entry out of it, rather than handing the reader itself around.
pub(super) struct Reader {
    file: File,
    index: HashMap<String, Entry>,
}

impl Reader {
    /// Reads the footer, checks the magic, and parses the index. Any structural problem — the
    /// file missing, too short for a footer, a bad magic, or an index that doesn't parse — is a
    /// `String` error, never a panic.
    pub(super) fn open(path: &Path) -> Result<Reader, String> {
        let mut file = File::open(path).map_err(|err| format!("vendor archive: {err}"))?;
        let len = file
            .metadata()
            .map_err(|err| format!("vendor archive: {err}"))?
            .len();
        if len < FOOTER_LEN {
            return Err("vendor archive: file shorter than its footer".to_string());
        }

        let mut footer = [0u8; FOOTER_LEN as usize];
        file.seek(SeekFrom::Start(len - FOOTER_LEN))
            .map_err(|err| format!("vendor archive: {err}"))?;
        file.read_exact(&mut footer)
            .map_err(|err| format!("vendor archive: {err}"))?;
        let (index_offset_bytes, magic) = footer.split_at(8);
        if magic != MAGIC {
            return Err("vendor archive: bad magic".to_string());
        }
        let index_offset = u64::from_le_bytes(index_offset_bytes.try_into().unwrap());
        if index_offset > len - FOOTER_LEN {
            return Err("vendor archive: index offset past the footer".to_string());
        }

        let index_len = (len - FOOTER_LEN - index_offset) as usize;
        file.seek(SeekFrom::Start(index_offset))
            .map_err(|err| format!("vendor archive: {err}"))?;
        let mut index_bytes = vec![0u8; index_len];
        file.read_exact(&mut index_bytes)
            .map_err(|err| format!("vendor archive: {err}"))?;
        let raw: Vec<(String, u64, u32, u32)> = serde_json::from_slice(&index_bytes)
            .map_err(|err| format!("vendor archive: bad index: {err}"))?;
        let index = raw
            .into_iter()
            .map(|(path, offset, clen, ulen)| (path, Entry { offset, clen, ulen }))
            .collect();

        Ok(Reader { file, index })
    }

    /// Seeks to `path`'s entry, reads its `clen` compressed bytes, and inflates them. `None` for
    /// a name with no entry, and for an entry whose inflated length doesn't match its recorded
    /// `ulen` — a truncated or corrupt entry never comes back as a short body.
    pub(super) fn read(&mut self, path: &str) -> Option<Vec<u8>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::DeflateEncoder;
    use std::io::Write as _;

    fn deflate(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn reads_two_entries_and_answers_none_for_a_third() {
        let a = b"hello world".to_vec();
        let b = b"a second entry, a bit longer than the first one".to_vec();
        let a_deflated = deflate(&a);
        let b_deflated = deflate(&b);

        let mut bytes = Vec::new();
        let a_offset = bytes.len() as u64;
        bytes.extend_from_slice(&a_deflated);
        let b_offset = bytes.len() as u64;
        bytes.extend_from_slice(&b_deflated);

        let index = serde_json::to_vec(&vec![
            (
                "a.txt".to_string(),
                a_offset,
                a_deflated.len() as u32,
                a.len() as u32,
            ),
            (
                "dir/b.txt".to_string(),
                b_offset,
                b_deflated.len() as u32,
                b.len() as u32,
            ),
        ])
        .unwrap();
        let index_offset = bytes.len() as u64;
        bytes.extend_from_slice(&index);
        bytes.extend_from_slice(&index_offset.to_le_bytes());
        bytes.extend_from_slice(MAGIC);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bundle.bin");
        std::fs::write(&path, &bytes).unwrap();

        let mut reader = Reader::open(&path).unwrap();
        assert_eq!(reader.read("a.txt"), Some(a));
        assert_eq!(reader.read("dir/b.txt"), Some(b));
        assert_eq!(reader.read("missing.txt"), None, "an absent name is None");
    }
}
