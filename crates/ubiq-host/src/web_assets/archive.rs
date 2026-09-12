//! The `.bundle` container a vendor bundle is written into: one file instead of a directory of
//! hundreds, because a filesystem that can create 554 small files is slower than one that writes
//! 554 spans of the same file, and it is the interface's job to read those spans back, not to
//! stat a tree.
//!
//! Layout, in order:
//!
//! ```text
//! [ deflate(entry 0 bytes) ][ deflate(entry 1) ] … [ index JSON ][ u64 LE index offset ][ magic ]
//! ```
//!
//! Every entry body is raw deflate ([`flate2::write::DeflateEncoder`], [`Compression::fast`] —
//! level 1: these bytes are read over loopback once they land, not shipped anywhere, so the only
//! cost worth paying is the write itself). The index is UTF-8 JSON, an array of
//! `["<path>", offset, compressed_len, uncompressed_len]` tuples, each offset relative to the
//! start of the file and each path the manifest's [`super::manifest::Entry::path`] exactly
//! (forward slashes, no leading slash). The footer is exactly 16 bytes: the index offset as a
//! little-endian `u64`, then the 8-byte magic `b"UBIQBND1"`.
//!
//! This module only writes. Nothing here reads a `.bundle` back — that is the interface's half,
//! over its own loopback origin.

use std::fs::File;
use std::io::{self, BufWriter, Write};

use flate2::Compression;
use flate2::write::DeflateEncoder;

/// The footer's magic, naming the format so a stray file is never mistaken for one.
const MAGIC: &[u8; 8] = b"UBIQBND1";

/// One entry in the index: its path, where its compressed body starts, and its two lengths.
type IndexEntry = (String, u64, u64, u64);

/// Appends entries to a `.bundle` file, tracking its own write offset rather than seeking —
/// simpler than a `Seek` bound, and every caller here already has a plain file opened for
/// writing.
pub struct Writer {
    out: BufWriter<File>,
    offset: u64,
    index: Vec<IndexEntry>,
}

impl Writer {
    pub fn new(file: File) -> Self {
        Self {
            out: BufWriter::new(file),
            offset: 0,
            index: Vec::new(),
        }
    }

    /// Deflate `bytes` and append them as one entry, recording `path` against the span they
    /// land in.
    pub fn write_entry(&mut self, path: &str, bytes: &[u8]) -> io::Result<()> {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(bytes)?;
        let compressed = encoder.finish()?;
        self.out.write_all(&compressed)?;
        self.index.push((
            path.to_string(),
            self.offset,
            compressed.len() as u64,
            bytes.len() as u64,
        ));
        self.offset += compressed.len() as u64;
        Ok(())
    }

    /// Write the index and the footer, and flush every byte to disk. The caller renames the file
    /// into place after this returns — that rename is the only proof the bundle is complete.
    pub fn finish(mut self) -> io::Result<()> {
        let index_offset = self.offset;
        let index_json = serde_json::to_vec(&self.index).map_err(io::Error::other)?;
        self.out.write_all(&index_json)?;
        self.out.write_all(&index_offset.to_le_bytes())?;
        self.out.write_all(MAGIC)?;
        self.out.flush()?;
        self.out.get_ref().sync_all()?;
        Ok(())
    }
}
