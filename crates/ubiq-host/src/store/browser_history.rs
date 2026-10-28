//! One project's browsing history on disk: `<data dir>/local/browser-history.json`.
//!
//! Per-machine and derived from what the user happened to open, so it sits under
//! [`super::project_dir::LOCAL`] and never travels with a project-managed `.ubiq/`. One file per
//! project, rewritten whole on every change through [`write_atomic`] — a thousand rows is a few
//! hundred kilobytes at worst, and the writes happen on the browser worker's thread, never the
//! coordinator's (`crate::browser`).
//!
//! The durability rule is view state's, not the catalogue's: losing it costs a list of links. A
//! file that will not parse is still preserved aside rather than truncated, because that costs one
//! rename; a file from a newer Ubiq is read as empty and **never overwritten**.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ubiq_proto::browser::{BrowserHistoryEntry, HISTORY_CAP};

use super::project_dir::ProjectData;
use crate::atomic::{preserve_aside, write_atomic};

/// The file's name inside the project's `local/` directory.
pub const FILE: &str = "browser-history.json";

/// The newest format this Ubiq writes and understands.
pub const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct HistoryFile {
    version: u32,
    #[serde(default)]
    entries: Vec<BrowserHistoryEntry>,
}

/// Where one project's history lives.
pub fn path(data: &ProjectData) -> PathBuf {
    data.local().join(FILE)
}

/// What a load found: the rows, and whether this Ubiq may write the file back.
pub struct Loaded {
    pub entries: Vec<BrowserHistoryEntry>,
    /// `false` for a file a newer Ubiq wrote: it is shown as empty and left untouched.
    pub writable: bool,
}

/// Read one project's history. A missing file is an empty history; an unreadable one is moved
/// aside and starts empty; a too-new one is empty and read-only.
pub fn load(path: &Path) -> Loaded {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(path = %path.display(), %error, "browser history unreadable");
            }
            return Loaded {
                entries: Vec::new(),
                writable: true,
            };
        }
    };
    match serde_json::from_str::<HistoryFile>(&raw) {
        Ok(file) if file.version <= VERSION => {
            let mut entries = file.entries;
            entries.sort_by(|a, b| b.last_visited.cmp(&a.last_visited));
            // One row per URL, the newest kept: a hand-edited or merged file may repeat one, and
            // `record` assumes it never does.
            let mut seen = std::collections::HashSet::new();
            entries.retain(|entry| seen.insert(entry.url.clone()));
            entries.truncate(HISTORY_CAP);
            Loaded {
                entries,
                writable: true,
            }
        }
        Ok(file) => {
            tracing::warn!(
                path = %path.display(),
                "browser history is version {}, and this Ubiq understands {VERSION}; leaving it untouched",
                file.version
            );
            Loaded {
                entries: Vec::new(),
                writable: false,
            }
        }
        Err(error) => {
            let preserved = preserve_aside(path, Utc::now()).ok();
            tracing::warn!(
                path = %path.display(),
                ?preserved,
                %error,
                "browser history would not parse; preserved aside and starting empty"
            );
            Loaded {
                entries: Vec::new(),
                writable: true,
            }
        }
    }
}

/// Write one project's history whole.
pub fn save(path: &Path, entries: &[BrowserHistoryEntry]) -> std::io::Result<()> {
    let file = HistoryFile {
        version: VERSION,
        entries: entries.to_vec(),
    };
    let bytes = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
    write_atomic(path, &bytes)
}

/// Record one visit: bump the URL's row and move it to the top, or add a new row there, then cap
/// the list. `false` when there was nothing to record — a blank URL.
pub fn record(
    entries: &mut Vec<BrowserHistoryEntry>,
    url: &str,
    title: &str,
    at: DateTime<Utc>,
) -> bool {
    let url = url.trim();
    if url.is_empty() {
        return false;
    }
    let entry = match entries.iter().position(|entry| entry.url == url) {
        Some(at_index) => {
            let mut entry = entries.remove(at_index);
            entry.visits = entry.visits.saturating_add(1);
            if !title.trim().is_empty() {
                entry.title = title.to_string();
            }
            entry.last_visited = at;
            entry
        }
        None => BrowserHistoryEntry {
            url: url.to_string(),
            title: title.to_string(),
            visits: 1,
            last_visited: at,
        },
    };
    entries.insert(0, entry);
    entries.truncate(HISTORY_CAP);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    #[test]
    fn a_revisit_moves_the_row_to_the_top_and_counts_it() {
        let mut entries = Vec::new();
        assert!(record(&mut entries, "https://a", "A", at(1)));
        assert!(record(&mut entries, "https://b", "B", at(2)));
        assert!(record(&mut entries, "https://a", "", at(3)));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].url, "https://a");
        assert_eq!(entries[0].visits, 2);
        assert_eq!(entries[0].title, "A");
        assert_eq!(entries[0].last_visited, at(3));
        assert!(!record(&mut entries, "  ", "blank", at(4)));
    }

    #[test]
    fn the_list_is_capped() {
        let mut entries = Vec::new();
        for i in 0..(HISTORY_CAP + 5) {
            record(&mut entries, &format!("https://{i}"), "", at(i as i64));
        }
        assert_eq!(entries.len(), HISTORY_CAP);
        assert_eq!(entries[0].url, format!("https://{}", HISTORY_CAP + 4));
    }

    #[test]
    fn a_saved_history_loads_back_and_a_corrupt_one_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("local").join(FILE);
        let mut entries = Vec::new();
        record(&mut entries, "https://a", "A", at(1));
        save(&file, &entries).unwrap();
        let loaded = load(&file);
        assert!(loaded.writable);
        assert_eq!(loaded.entries, entries);

        std::fs::write(&file, b"not json").unwrap();
        let loaded = load(&file);
        assert!(loaded.entries.is_empty());
        assert!(loaded.writable);
        assert!(!file.exists());
    }

    #[test]
    fn a_too_new_file_is_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        std::fs::write(&file, br#"{"version": 99, "entries": []}"#).unwrap();
        assert!(!load(&file).writable);
    }
}
