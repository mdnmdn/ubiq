//! The on-disk harness catalogue cache: model + reasoning-level answers, keyed on the harness
//! binary's own version string.
//!
//! Modelled line for line on [`super::file::FileProjectStore`] — one TOML file, an `RwLock` live
//! copy, a `durable: AtomicBool` degradation — with one deliberate difference: this is a cache,
//! not a catalogue. A missing, corrupt, or too-new-version file costs nothing to lose, so it loads
//! as empty (never an error) and the next successful probe rewrites it.

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::atomic::write_atomic;

/// The cache format this Ubiq writes and understands. Bumped only if the shape of
/// [`CachedModel`]/[`CachedLevel`] changes in a way an old file can't be read as; unrelated to a
/// harness binary's own `version` string, which is per-entry, not per-file.
pub const HARNESS_CACHE_VERSION: u32 = 1;

/// One reasoning-effort level a model accepts, in the harness's own vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedLevel {
    pub value: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One model, with whatever reasoning levels it accepts folded in — the two probes
/// (`discover_models` + `discover_thinking`) are joined here, once, so the picker reads one
/// record instead of two.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedModel {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub default: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<CachedLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_level: Option<String>,
}

/// One cached answer, keyed on the three things that can change it: which harness, which
/// identity asked, and which build of the binary answered.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    harness: String,
    account: String,
    version: String,
    #[serde(default)]
    model: Vec<CachedModel>,
}

/// The whole file. `version` is at the top so a future migration has a hook to read, mirroring
/// [`super::file::CatalogueFile`].
#[derive(Debug, Default, Serialize, Deserialize)]
struct HarnessCacheFile {
    version: u32,
    #[serde(default, rename = "entry", skip_serializing_if = "Vec::is_empty")]
    entries: Vec<CacheEntry>,
}

/// The harness catalogue cache, as one TOML file under `<config_root>/cache/harness-models.toml`.
pub struct FileHarnessCache {
    path: PathBuf,
    /// The live cache. A read that misses in memory is a miss, full stop — there is no on-demand
    /// disk read per lookup, the whole file is loaded once at construction.
    entries: RwLock<Vec<CacheEntry>>,
    /// Cleared by the first failed write. A cache write failing is not worth telling anyone about
    /// twice: the answer just costs a re-probe next time, forever, until the process restarts.
    durable: AtomicBool,
}

impl FileHarnessCache {
    /// Open the cache at `path`, loading whatever is already there. Missing, corrupt, or
    /// too-new-version content all load as empty — see [`Self::load`].
    pub fn new(path: PathBuf) -> Self {
        let cache = Self {
            path,
            entries: RwLock::new(Vec::new()),
            durable: AtomicBool::new(true),
        };
        cache.load();
        cache
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// (Re)read the file from disk into memory. A missing file, one that doesn't parse, or one
    /// stamped with a `version` newer than [`HARNESS_CACHE_VERSION`] all leave the in-memory copy
    /// empty rather than erroring — this is a cache, not a catalogue, and the next successful
    /// [`Self::put`] simply overwrites whatever was there.
    pub fn load(&self) {
        let loaded = std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|raw| toml::from_str::<HarnessCacheFile>(&raw).ok())
            .filter(|file| file.version <= HARNESS_CACHE_VERSION)
            .map(|file| file.entries)
            .unwrap_or_default();
        *self.entries.write().unwrap_or_else(|e| e.into_inner()) = loaded;
    }

    /// The cached models for `(harness, account)`, only when the stored answer came from the same
    /// binary `version` — a different version is a miss, not a stale hit, because invalidation
    /// here is the version string and nothing else (no TTL).
    pub fn get(&self, harness: &str, account: &str, version: &str) -> Option<Vec<CachedModel>> {
        self.entries
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|e| e.harness == harness && e.account == account && e.version == version)
            .map(|e| e.model.clone())
    }

    /// Store `models` for `(harness, account, version)`, replacing any prior answer for that
    /// harness/account. A no-op for an empty `models`: an empty answer is a failed probe wearing
    /// a success, and caching it would hide a harness that logs in a minute later.
    pub fn put(&self, harness: &str, account: &str, version: &str, models: Vec<CachedModel>) {
        if models.is_empty() {
            return;
        }
        {
            let mut entries = self.entries.write().unwrap_or_else(|e| e.into_inner());
            match entries
                .iter_mut()
                .find(|e| e.harness == harness && e.account == account)
            {
                Some(existing) => {
                    existing.version = version.to_string();
                    existing.model = models;
                }
                None => entries.push(CacheEntry {
                    harness: harness.to_string(),
                    account: account.to_string(),
                    version: version.to_string(),
                    model: models,
                }),
            }
        }
        if !self.durable.load(Ordering::Relaxed) {
            return;
        }
        self.flush();
    }

    /// Rewrite the file from what is in memory. Failure just flips `durable` — losing this cache
    /// loses nothing but the next probe's shortcut.
    fn flush(&self) {
        let entries = self.entries.read().unwrap_or_else(|e| e.into_inner());
        let file = HarnessCacheFile {
            version: HARNESS_CACHE_VERSION,
            entries: entries.clone(),
        };
        drop(entries);

        let Ok(body) = toml::to_string_pretty(&file) else {
            return;
        };
        if write_atomic(&self.path, body.as_bytes()).is_err() {
            self.durable.store(false, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("cache").join("harness-models.toml")
    }

    fn one_model(id: &str) -> Vec<CachedModel> {
        vec![CachedModel {
            id: id.to_string(),
            description: None,
            default: true,
            levels: vec![CachedLevel {
                value: "high".to_string(),
                label: "High".to_string(),
                description: None,
            }],
            default_level: None,
        }]
    }

    #[test]
    fn put_then_get_round_trips() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = FileHarnessCache::new(cache_path(&dir));
        cache.put("claude-code", "default", "2.1.261", one_model("sonnet"));

        let got = cache.get("claude-code", "default", "2.1.261");
        assert_eq!(got, Some(one_model("sonnet")));
    }

    #[test]
    fn a_different_version_misses() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = FileHarnessCache::new(cache_path(&dir));
        cache.put("claude-code", "default", "2.1.261", one_model("sonnet"));

        assert_eq!(cache.get("claude-code", "default", "2.1.262"), None);
    }

    #[test]
    fn a_different_account_misses() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = FileHarnessCache::new(cache_path(&dir));
        cache.put("claude-code", "default", "2.1.261", one_model("sonnet"));

        assert_eq!(cache.get("claude-code", "work", "2.1.261"), None);
    }

    #[test]
    fn put_with_empty_models_leaves_no_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = cache_path(&dir);
        let cache = FileHarnessCache::new(path.clone());
        cache.put("claude-code", "default", "2.1.261", Vec::new());

        assert!(!path.exists());
        assert_eq!(cache.get("claude-code", "default", "2.1.261"), None);
    }

    #[test]
    fn a_corrupt_file_loads_empty_and_still_accepts_a_put() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = cache_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not valid toml {{{").unwrap();

        let cache = FileHarnessCache::new(path);
        assert_eq!(cache.get("claude-code", "default", "2.1.261"), None);

        cache.put("claude-code", "default", "2.1.261", one_model("sonnet"));
        assert_eq!(
            cache.get("claude-code", "default", "2.1.261"),
            Some(one_model("sonnet"))
        );
    }

    #[test]
    fn a_too_new_version_file_loads_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = cache_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("version = {}\n", HARNESS_CACHE_VERSION + 1)).unwrap();

        let cache = FileHarnessCache::new(path);
        assert_eq!(cache.get("claude-code", "default", "2.1.261"), None);
    }

    #[test]
    fn a_missing_file_loads_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = FileHarnessCache::new(cache_path(&dir));
        assert_eq!(cache.get("claude-code", "default", "2.1.261"), None);
    }

    #[test]
    fn put_persists_to_disk_and_a_fresh_cache_reads_it_back() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = cache_path(&dir);
        {
            let cache = FileHarnessCache::new(path.clone());
            cache.put(
                "codex",
                "default",
                "codex-cli 0.142.5",
                one_model("gpt-5-codex"),
            );
        }

        let reopened = FileHarnessCache::new(path);
        assert_eq!(
            reopened.get("codex", "default", "codex-cli 0.142.5"),
            Some(one_model("gpt-5-codex"))
        );
    }
}
