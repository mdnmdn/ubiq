//! The blob cache: what a cell was cut from, kept for `get_blob`.
//!
//! A query answers with every text cell cut to the caller's `max_field_size` and every binary cell
//! replaced by a marker, `[blob:<id>:<len>]`. The full value goes here, so the agent that wants
//! the rest asks for it by id instead of re-running the query.
//!
//! **Keyed `(agent_key, id)`.** One agent can never read another's blob, even by guessing an id,
//! and an id means nothing without its owner. Ids are `b` plus the base-36 of a counter.
//!
//! **Bounded three ways**: [`MAX_ENTRIES`], [`MAX_TOTAL_BYTES`] and [`MAX_BLOB_BYTES`] per blob (a
//! larger value is stored cut to that, and the marker reports the stored length). A blob expires
//! [`TTL`] after it was last *read*, scanned on every access, and the least recently used goes
//! first when a bound is hit.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

pub const MAX_ENTRIES: usize = 1024;
pub const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_BLOB_BYTES: usize = 16 * 1024 * 1024;
pub const TTL: Duration = Duration::from_secs(10 * 60);

/// What was cut from a cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blob {
    Text(String),
    Bytes(Vec<u8>),
}

impl Blob {
    /// Characters for text, bytes for binary: the number a marker carries.
    pub fn len(&self) -> usize {
        match self {
            Blob::Text(text) => text.chars().count(),
            Blob::Bytes(bytes) => bytes.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn stored_bytes(&self) -> usize {
        match self {
            Blob::Text(text) => text.len(),
            Blob::Bytes(bytes) => bytes.len(),
        }
    }
}

/// A stored blob's id and the length its marker reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub id: String,
    pub len: usize,
}

/// One window onto a blob: `offset`/`length` are in characters for text and bytes for binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slice {
    pub total: usize,
    pub offset: usize,
    pub returned: usize,
    pub binary: bool,
    /// The text, or the base64 of the bytes.
    pub data: String,
}

struct Entry {
    blob: Arc<Blob>,
    last_read: Instant,
    /// Monotonic touch counter: the least recently used is the lowest.
    tick: u64,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<(String, String), Entry>,
    bytes: usize,
    next_id: u64,
    tick: u64,
}

pub struct Blobs {
    inner: Mutex<Inner>,
    max_entries: usize,
    max_total: usize,
    ttl: Duration,
}

impl Default for Blobs {
    fn default() -> Self {
        Self::with_bounds(MAX_ENTRIES, MAX_TOTAL_BYTES, TTL)
    }
}

impl Blobs {
    pub fn with_bounds(max_entries: usize, max_total: usize, ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            max_entries,
            max_total,
            ttl,
        }
    }

    pub fn put(&self, agent: &str, blob: Blob) -> Stored {
        self.put_at(agent, blob, Instant::now())
    }

    pub fn get(&self, agent: &str, id: &str) -> Option<Arc<Blob>> {
        self.get_at(agent, id, Instant::now())
    }

    /// `length` is clamped to what remains; an `offset` past the end returns nothing.
    pub fn slice(&self, agent: &str, id: &str, offset: usize, length: usize) -> Option<Slice> {
        let blob = self.get(agent, id)?;
        Some(slice_of(&blob, offset, length))
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .map(|inner| inner.entries.len())
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn put_at(&self, agent: &str, blob: Blob, now: Instant) -> Stored {
        let blob = cut_to_cap(blob);
        let len = blob.len();
        let size = blob.stored_bytes();
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        self.expire(&mut inner, now);
        inner.next_id += 1;
        let id = format!("b{}", base36(inner.next_id));
        inner.tick += 1;
        let tick = inner.tick;
        inner.bytes += size;
        inner.entries.insert(
            (agent.to_string(), id.clone()),
            Entry {
                blob: Arc::new(blob),
                last_read: now,
                tick,
            },
        );
        // Evict the least recently used until both bounds hold. The new entry is the most recent,
        // so it goes last — and a single blob never exceeds the total, by `MAX_BLOB_BYTES`.
        while inner.entries.len() > self.max_entries || inner.bytes > self.max_total {
            let Some(oldest) = inner
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.tick)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(gone) = inner.entries.remove(&oldest) {
                inner.bytes -= gone.blob.stored_bytes();
            }
        }
        Stored { id, len }
    }

    fn get_at(&self, agent: &str, id: &str, now: Instant) -> Option<Arc<Blob>> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        self.expire(&mut inner, now);
        inner.tick += 1;
        let tick = inner.tick;
        let entry = inner
            .entries
            .get_mut(&(agent.to_string(), id.to_string()))?;
        entry.last_read = now;
        entry.tick = tick;
        Some(Arc::clone(&entry.blob))
    }

    fn expire(&self, inner: &mut Inner, now: Instant) {
        let ttl = self.ttl;
        let mut freed = 0;
        inner.entries.retain(|_, entry| {
            let keep = now.saturating_duration_since(entry.last_read) < ttl;
            if !keep {
                freed += entry.blob.stored_bytes();
            }
            keep
        });
        inner.bytes -= freed;
    }
}

fn slice_of(blob: &Blob, offset: usize, length: usize) -> Slice {
    match blob {
        Blob::Text(text) => {
            let total = blob.len();
            let data: String = text.chars().skip(offset).take(length).collect();
            Slice {
                total,
                offset,
                returned: data.chars().count(),
                binary: false,
                data,
            }
        }
        Blob::Bytes(bytes) => {
            let start = offset.min(bytes.len());
            let end = start.saturating_add(length).min(bytes.len());
            Slice {
                total: bytes.len(),
                offset,
                returned: end - start,
                binary: true,
                data: STANDARD.encode(&bytes[start..end]),
            }
        }
    }
}

fn cut_to_cap(blob: Blob) -> Blob {
    match blob {
        Blob::Bytes(mut bytes) if bytes.len() > MAX_BLOB_BYTES => {
            bytes.truncate(MAX_BLOB_BYTES);
            Blob::Bytes(bytes)
        }
        Blob::Text(mut text) if text.len() > MAX_BLOB_BYTES => {
            let mut end = MAX_BLOB_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            Blob::Text(text)
        }
        other => other,
    }
}

fn base36(mut n: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Blob {
        Blob::Text(s.to_string())
    }

    #[test]
    fn a_blob_comes_back_by_id_and_owner() {
        let blobs = Blobs::default();
        let stored = blobs.put("a", text("héllo wörld"));
        assert_eq!(stored.len, 11);
        assert!(stored.id.starts_with('b'));
        assert_eq!(*blobs.get("a", &stored.id).unwrap(), text("héllo wörld"));
    }

    #[test]
    fn agents_cannot_read_each_others_blobs() {
        let blobs = Blobs::default();
        let stored = blobs.put("a", text("secret"));
        assert!(blobs.get("b", &stored.id).is_none());
        assert!(blobs.get("a", &stored.id).is_some());
        // The same id for another agent is a different blob.
        let other = blobs.put("b", text("other"));
        assert_ne!(other.id, stored.id);
    }

    #[test]
    fn the_least_recently_used_goes_first() {
        let blobs = Blobs::with_bounds(2, MAX_TOTAL_BYTES, TTL);
        let one = blobs.put("a", text("1"));
        let two = blobs.put("a", text("2"));
        // Reading one makes two the oldest.
        assert!(blobs.get("a", &one.id).is_some());
        let three = blobs.put("a", text("3"));
        assert!(blobs.get("a", &two.id).is_none());
        assert!(blobs.get("a", &one.id).is_some());
        assert!(blobs.get("a", &three.id).is_some());
        assert_eq!(blobs.len(), 2);
    }

    #[test]
    fn the_total_size_bound_evicts_too() {
        let blobs = Blobs::with_bounds(100, 10, TTL);
        let a = blobs.put("a", text("123456"));
        let b = blobs.put("a", text("123456"));
        assert!(blobs.get("a", &a.id).is_none());
        assert!(blobs.get("a", &b.id).is_some());
    }

    #[test]
    fn an_entry_expires_a_ttl_after_it_was_last_read() {
        let blobs = Blobs::with_bounds(10, MAX_TOTAL_BYTES, Duration::from_secs(60));
        let t0 = Instant::now();
        let stored = blobs.put_at("a", text("x"), t0);
        // A read at 50s keeps it alive past the original 60s.
        assert!(
            blobs
                .get_at("a", &stored.id, t0 + Duration::from_secs(50))
                .is_some()
        );
        assert!(
            blobs
                .get_at("a", &stored.id, t0 + Duration::from_secs(100))
                .is_some()
        );
        // No read for a full ttl and it is gone, and its bytes are released.
        assert!(
            blobs
                .get_at("a", &stored.id, t0 + Duration::from_secs(161))
                .is_none()
        );
        assert_eq!(blobs.inner.lock().unwrap().bytes, 0);
    }

    #[test]
    fn a_text_slice_counts_characters_and_binary_comes_back_base64() {
        let blobs = Blobs::default();
        let t = blobs.put("a", text("aébcd"));
        let slice = blobs.slice("a", &t.id, 1, 2).unwrap();
        assert_eq!(
            (
                slice.data.as_str(),
                slice.total,
                slice.returned,
                slice.binary
            ),
            ("éb", 5, 2, false)
        );
        let past = blobs.slice("a", &t.id, 9, 2).unwrap();
        assert_eq!((past.data.as_str(), past.returned), ("", 0));

        let b = blobs.put("a", Blob::Bytes(vec![0, 1, 2, 3, 255]));
        let slice = blobs.slice("a", &b.id, 1, 100).unwrap();
        assert_eq!(slice.data, STANDARD.encode([1, 2, 3, 255]));
        assert_eq!((slice.total, slice.returned, slice.binary), (5, 4, true));
    }

    #[test]
    fn a_blob_over_the_per_blob_cap_is_stored_cut() {
        let blobs = Blobs::default();
        let stored = blobs.put("a", Blob::Bytes(vec![7; MAX_BLOB_BYTES + 5]));
        assert_eq!(stored.len, MAX_BLOB_BYTES);
    }

    #[test]
    fn ids_are_base36() {
        assert_eq!(base36(0), "0");
        assert_eq!(base36(35), "z");
        assert_eq!(base36(36), "10");
    }
}
