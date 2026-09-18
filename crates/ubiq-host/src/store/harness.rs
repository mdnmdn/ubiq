//! The on-disk harness catalogue cache: model + reasoning-level answers keyed on the harness
//! binary's own version string, and the ACP capabilities an agent last stated.
//!
//! Modelled line for line on [`super::file::FileProjectStore`] — one TOML file, an `RwLock` live
//! copy, a `durable: AtomicBool` degradation — with one deliberate difference: this is a cache,
//! not a catalogue. A missing, corrupt, or too-new-version file costs nothing to lose, so it loads
//! as empty (never an error) and the next successful probe rewrites it.

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use ubiq_proto::acp::AcpCapabilitiesRecord;

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

/// What an ACP agent on this harness said it can do, and when it said it.
///
/// One row per harness — not per account and not per version. An ACP agent answers `initialize`
/// the same way whoever is signed in, and an answer from an older build is still the last true
/// answer, so a version mismatch is not a miss here the way it is for [`CacheEntry`]. Which build
/// answered is inside the record, stated by the agent itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AcpEntry {
    harness: String,
    #[serde(default)]
    discovered_ms: i64,
    /// The record itself, in the wire's own vocabulary — the host maps the library's answer onto
    /// it once, at capture, so nothing re-maps on the way out.
    capabilities: AcpCapabilitiesRecord,
}

/// The model and thinking level a harness was last actually launched with — not the harness's own
/// default, and not account-scoped (the user asked for per-harness). Empty means "no flag was
/// passed", the same convention `crate::coordinator`'s `PendingConversation.chosen_model` and
/// `chosen_thinking` use.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LastUsedEntry {
    harness: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    thinking: String,
}

/// The whole file. `version` is at the top so a future migration has a hook to read, mirroring
/// [`super::file::CatalogueFile`].
#[derive(Debug, Default, Serialize, Deserialize)]
struct HarnessCacheFile {
    version: u32,
    #[serde(default, rename = "entry", skip_serializing_if = "Vec::is_empty")]
    entries: Vec<CacheEntry>,
    #[serde(default, rename = "last_used", skip_serializing_if = "Vec::is_empty")]
    last_used: Vec<LastUsedEntry>,
    #[serde(default, rename = "acp", skip_serializing_if = "Vec::is_empty")]
    acp: Vec<AcpEntry>,
}

/// The harness catalogue cache, as one TOML file under `<config_root>/cache/harness-models.toml`.
pub struct FileHarnessCache {
    path: PathBuf,
    /// The live cache. A read that misses in memory is a miss, full stop — there is no on-demand
    /// disk read per lookup, the whole file is loaded once at construction.
    entries: RwLock<Vec<CacheEntry>>,
    /// The last model/thinking a harness actually launched with, one row per harness (not
    /// account: the user asked to remember per-harness only).
    last_used: RwLock<Vec<LastUsedEntry>>,
    /// What each ACP harness last said it can do, one row per harness. See [`AcpEntry`].
    acp: RwLock<Vec<AcpEntry>>,
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
            last_used: RwLock::new(Vec::new()),
            acp: RwLock::new(Vec::new()),
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
    /// [`Self::put`]/[`Self::set_last_used`] simply overwrites whatever was there.
    pub fn load(&self) {
        let file = std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|raw| toml::from_str::<HarnessCacheFile>(&raw).ok())
            .filter(|file| file.version <= HARNESS_CACHE_VERSION);
        *self.entries.write().unwrap_or_else(|e| e.into_inner()) = file
            .as_ref()
            .map(|file| file.entries.clone())
            .unwrap_or_default();
        *self.acp.write().unwrap_or_else(|e| e.into_inner()) = file
            .as_ref()
            .map(|file| file.acp.clone())
            .unwrap_or_default();
        *self.last_used.write().unwrap_or_else(|e| e.into_inner()) =
            file.map(|file| file.last_used).unwrap_or_default();
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

    /// The model and thinking level `harness` was last actually launched with, as
    /// `(model, thinking)` — either or both empty when nothing was chosen for that leg. `None`
    /// when this harness has never launched at all, which the caller reads the same way it reads
    /// an empty string: fall back to the harness default.
    pub fn last_used(&self, harness: &str) -> Option<(String, String)> {
        self.last_used
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|e| e.harness == harness)
            .map(|e| (e.model.clone(), e.thinking.clone()))
    }

    /// Record what `harness` was just launched with, replacing whatever it last recorded — one
    /// row per harness, no account leg (see [`LastUsedEntry`]). Called once, from
    /// `Coordinator::launch`, only after a launch actually happens.
    pub fn set_last_used(&self, harness: &str, model: &str, thinking: &str) {
        {
            let mut last_used = self.last_used.write().unwrap_or_else(|e| e.into_inner());
            match last_used.iter_mut().find(|e| e.harness == harness) {
                Some(existing) => {
                    existing.model = model.to_string();
                    existing.thinking = thinking.to_string();
                }
                None => last_used.push(LastUsedEntry {
                    harness: harness.to_string(),
                    model: model.to_string(),
                    thinking: thinking.to_string(),
                }),
            }
        }
        if !self.durable.load(Ordering::Relaxed) {
            return;
        }
        self.flush();
    }

    /// What `harness`'s ACP agent last said it can do, with the version that said it and the
    /// moment it did folded in. `None` where no ACP handshake on this harness has ever been seen —
    /// which is the permanent answer for a harness that speaks its own wire, and the temporary one
    /// for an ACP harness nothing has conversed with yet.
    pub fn acp_capabilities(&self, harness: &str) -> Option<AcpCapabilitiesRecord> {
        self.acp
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|e| e.harness == harness)
            .map(|e| {
                let mut record = e.capabilities.clone();
                record.discovered_ms = e.discovered_ms;
                record
            })
    }

    /// Record what `harness`'s ACP agent just said, replacing whatever it said before. Called once
    /// per ACP bridge, from the conversation pump, right after the handshake that produced it —
    /// there is no probe that asks this question on its own.
    pub fn set_acp_capabilities(
        &self,
        harness: &str,
        discovered_ms: i64,
        capabilities: AcpCapabilitiesRecord,
    ) {
        {
            let mut acp = self.acp.write().unwrap_or_else(|e| e.into_inner());
            match acp.iter_mut().find(|e| e.harness == harness) {
                Some(existing) => {
                    existing.discovered_ms = discovered_ms;
                    existing.capabilities = capabilities;
                }
                None => acp.push(AcpEntry {
                    harness: harness.to_string(),
                    discovered_ms,
                    capabilities,
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
        let last_used = self.last_used.read().unwrap_or_else(|e| e.into_inner());
        let acp = self.acp.read().unwrap_or_else(|e| e.into_inner());
        let file = HarnessCacheFile {
            version: HARNESS_CACHE_VERSION,
            entries: entries.clone(),
            last_used: last_used.clone(),
            acp: acp.clone(),
        };
        drop(entries);
        drop(last_used);
        drop(acp);

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

    #[test]
    fn nothing_is_remembered_for_a_harness_that_has_not_launched() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = FileHarnessCache::new(cache_path(&dir));
        assert_eq!(cache.last_used("claude-code"), None);
    }

    #[test]
    fn set_last_used_then_last_used_round_trips() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = FileHarnessCache::new(cache_path(&dir));
        cache.set_last_used("claude-code", "opus", "high");

        assert_eq!(
            cache.last_used("claude-code"),
            Some(("opus".to_string(), "high".to_string()))
        );
        // A different harness is unaffected.
        assert_eq!(cache.last_used("codex"), None);
    }

    #[test]
    fn set_last_used_replaces_the_harnesss_prior_answer() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = FileHarnessCache::new(cache_path(&dir));
        cache.set_last_used("claude-code", "opus", "high");
        cache.set_last_used("claude-code", "sonnet", "");

        assert_eq!(
            cache.last_used("claude-code"),
            Some(("sonnet".to_string(), String::new()))
        );
    }

    #[test]
    fn last_used_persists_to_disk_and_a_fresh_cache_reads_it_back() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = cache_path(&dir);
        {
            let cache = FileHarnessCache::new(path.clone());
            cache.set_last_used("codex", "gpt-5-codex", "medium");
        }

        let reopened = FileHarnessCache::new(path);
        assert_eq!(
            reopened.last_used("codex"),
            Some(("gpt-5-codex".to_string(), "medium".to_string()))
        );
    }

    /// The model cache and the last-used row share one file: a probe result and a launch pick
    /// both survive a reopen together.
    #[test]
    fn last_used_coexists_with_the_model_cache_in_the_same_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = cache_path(&dir);
        {
            let cache = FileHarnessCache::new(path.clone());
            cache.put("claude-code", "default", "2.1.261", one_model("sonnet"));
            cache.set_last_used("claude-code", "sonnet", "high");
        }

        let reopened = FileHarnessCache::new(path);
        assert_eq!(
            reopened.get("claude-code", "default", "2.1.261"),
            Some(one_model("sonnet"))
        );
        assert_eq!(
            reopened.last_used("claude-code"),
            Some(("sonnet".to_string(), "high".to_string()))
        );
    }

    fn one_record() -> AcpCapabilitiesRecord {
        AcpCapabilitiesRecord {
            protocol_version: 1,
            agent: Some(ubiq_proto::acp::AcpImplementationRecord {
                name: "opencode".to_string(),
                title: None,
                version: Some("1.18.28".to_string()),
            }),
            groups: vec![ubiq_proto::acp::AcpCapabilityGroupRecord {
                label: "Sessions".to_string(),
                entries: vec![ubiq_proto::acp::AcpCapabilityRecord {
                    id: "loadSession".to_string(),
                    label: "Load session".to_string(),
                    supported: true,
                    description: String::new(),
                }],
            }],
            auth_methods: Vec::new(),
            discovered_ms: 0,
        }
    }

    #[test]
    fn acp_capabilities_round_trip_through_the_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = FileHarnessCache::new(cache_path(&dir));
        cache.set_acp_capabilities("opencode", 1_700_000_000_000, one_record());

        let reopened = FileHarnessCache::new(cache_path(&dir));
        let got = reopened.acp_capabilities("opencode").expect("a record");
        assert_eq!(got.protocol_version, 1);
        assert_eq!(got.discovered_ms, 1_700_000_000_000);
        assert_eq!(got.groups[0].entries[0].id, "loadSession");
    }

    /// The record is keyed on the harness alone, so a second handshake replaces the first rather
    /// than accumulating a row per run.
    #[test]
    fn a_second_handshake_replaces_the_record() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = FileHarnessCache::new(cache_path(&dir));
        cache.set_acp_capabilities("opencode", 1, one_record());
        let mut later = one_record();
        later.groups[0].entries[0].supported = false;
        cache.set_acp_capabilities("opencode", 2, later);

        let got = cache.acp_capabilities("opencode").expect("a record");
        assert_eq!(got.discovered_ms, 2);
        assert!(!got.groups[0].entries[0].supported);
    }

    /// A harness nothing has conversed with has no record, which is the answer the wire carries.
    #[test]
    fn a_harness_with_no_handshake_has_no_record() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = FileHarnessCache::new(cache_path(&dir));
        assert!(cache.acp_capabilities("claude-code").is_none());
    }
}
