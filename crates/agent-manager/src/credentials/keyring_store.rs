//! OS system credential store [`SecretStore`] (`engine = "keyring"`), via the
//! [`keyring`] crate — a sibling to [`super::OsSecretStore`] (`engine = "os"`),
//! not a replacement.
//!
//! Same shape as [`super::OsSecretStore`]: one entry per [`CredentialId`],
//! whose value is the JSON of its blobs (the `{name, rel_path, bytes}` wire
//! shape), plus a small non-secret `<dir>/index.json` (just the
//! `(harness, name)` keys) backing [`SecretStore::list`]. `list`/`rename`/
//! blob-encoding are backend-agnostic; only get/set/delete of one string per
//! key goes through `keyring`.
//!
//! **Where this differs from `os`:** [`super::OsSecretStore`] on macOS shells
//! out to `/usr/bin/security` against a *custom keychain file* it creates
//! under the config dir — deliberately laundering access through a stable,
//! signed system binary so the stored items aren't bound to `am`'s own (often
//! ad-hoc, rebuilt-each-time) code identity. This engine instead calls the
//! platform's native credential API in-process via `keyring`, storing items
//! in the **OS system store** (the user's actual login keychain / Windows
//! Credential Manager / Secret Service collection) rather than a file under
//! `<config>/keychain`. That's simpler and needs no subprocess, but it comes
//! with real caveats worth stating plainly rather than glossing over:
//!
//! - **Windows:** silent — DPAPI-backed Credential Manager entries are
//!   readable by the same OS user with no prompt. Also note
//!   `CRED_MAX_CREDENTIAL_BLOB_SIZE` (about 2.5 KB): a credential whose
//!   JSON-encoded blob set exceeds that (e.g. a large Claude Code
//!   `.claude.json` identity file bundled alongside `.credentials.json`) can
//!   be rejected outright by Credential Manager. `os`/`files` don't have this
//!   ceiling.
//! - **Linux (Secret Service via zbus):** free only when a login keyring is
//!   already unlocked (a normal desktop session, generally auto-unlocked at
//!   login). Headless boxes, CI runners, and plain SSH sessions typically
//!   have no D-Bus session bus / no unlocked collection at all, so calls
//!   here fail outright rather than prompting — same "not always available"
//!   character as `secret-tool` in `os.rs`'s Linux draft, just surfaced
//!   through a different code path.
//! - **macOS:** may re-prompt ("`am` wants to access your keychain...") on
//!   *every* rebuild of an unsigned/ad-hoc-signed `am` binary, because the
//!   Keychain ACL an item is stored under binds to the storing process's
//!   (unstable, per-build) code identity — exactly the failure mode
//!   [`super::OsSecretStore`]'s custom-keychain-via-`security` design exists
//!   to avoid. A consistently-signed release build doesn't hit this; a
//!   locally `cargo build`'d one, rebuilt often, will.
//!
//! None of this makes the engine wrong to offer — an embedder that ships a
//! signed binary and only targets macOS/Windows, or that's fine with a
//! desktop Linux assumption, gets a real OS-encrypted store with one fewer
//! moving part (no subprocess, no bespoke keychain file). It's opt-in
//! (`engine = "keyring"`) precisely because those tradeoffs don't fit every
//! deployment.

use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::Result;

use super::{CredentialBlob, CredentialId, CredentialMeta, SecretStore};

/// Fixed keyring "account"/username for every entry — the service string
/// (see [`entry_service`]) already encodes `(harness, name)`, so the account
/// component doesn't need to vary.
const KEYRING_ACCOUNT: &str = "am";

/// One blob as serialized into an entry's secret value. Mirrors
/// [`super::os::OsSecretStore`]'s wire shape; serde's default `Vec<u8>`
/// encoding (a JSON array of numbers) keeps the value pure ASCII with no NUL.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct WireBlob {
    name: String,
    rel_path: PathBuf,
    bytes: Vec<u8>,
}

impl From<&CredentialBlob> for WireBlob {
    fn from(b: &CredentialBlob) -> Self {
        WireBlob {
            name: b.name.clone(),
            rel_path: b.rel_path.clone(),
            bytes: b.bytes.clone(),
        }
    }
}

impl From<WireBlob> for CredentialBlob {
    fn from(b: WireBlob) -> Self {
        CredentialBlob {
            name: b.name,
            rel_path: b.rel_path,
            bytes: b.bytes,
        }
    }
}

/// A `(harness, name)` key as recorded in the non-secret index file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct IndexKey {
    harness: String,
    name: String,
}

impl From<&CredentialId> for IndexKey {
    fn from(id: &CredentialId) -> Self {
        IndexKey {
            harness: id.harness.clone(),
            name: id.name.clone(),
        }
    }
}

/// A [`SecretStore`] backed by the OS system credential store, via the
/// [`keyring`] crate.
#[derive(Debug, Clone)]
pub struct KeyringSecretStore {
    /// Directory holding only the non-secret `index.json` — the credential
    /// bodies themselves live in the OS store, not under this directory.
    dir: PathBuf,
}

impl KeyringSecretStore {
    /// Create a store whose index file is `<dir>/index.json` (e.g.
    /// `<config>/keychain`, same directory [`super::OsSecretStore`] roots
    /// at).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        KeyringSecretStore { dir: dir.into() }
    }

    /// Encode a credential's blobs into one entry value (compact JSON).
    fn encode(blobs: &[CredentialBlob]) -> Result<String> {
        let wire: Vec<WireBlob> = blobs.iter().map(WireBlob::from).collect();
        serde_json::to_string(&wire).context("serializing credential blobs for the keyring store")
    }

    /// Decode an entry value back into blobs.
    fn decode(value: &str) -> Result<Vec<CredentialBlob>> {
        let wire: Vec<WireBlob> = serde_json::from_str(value)
            .context("parsing credential blobs from the keyring store")?;
        Ok(wire.into_iter().map(CredentialBlob::from).collect())
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("index.json")
    }

    fn read_index(&self) -> Result<Vec<IndexKey>> {
        let path = self.index_path();
        if !path.is_file() {
            return Ok(Vec::new());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    fn write_index(&self, keys: &[IndexKey]) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating {}", self.dir.display()))?;
        let path = self.index_path();
        let body = serde_json::to_string_pretty(keys).context("serializing keyring store index")?;
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    fn index_add(&self, id: &CredentialId) -> Result<()> {
        let mut keys = self.read_index()?;
        let key = IndexKey::from(id);
        if !keys.contains(&key) {
            keys.push(key);
            self.write_index(&keys)?;
        }
        Ok(())
    }

    fn index_remove(&self, id: &CredentialId) -> Result<()> {
        let mut keys = self.read_index()?;
        let key = IndexKey::from(id);
        let before = keys.len();
        keys.retain(|k| k != &key);
        if keys.len() != before {
            self.write_index(&keys)?;
        }
        Ok(())
    }

    /// Build the `keyring::Entry` for `id`. `Entry::new` only allocates a
    /// handle (backed by the crate's default-registered platform store —
    /// see the module-level API note); it does no I/O itself, so failures
    /// here are about entry construction, not store availability.
    fn entry(id: &CredentialId) -> Result<keyring::Entry> {
        keyring::Entry::new(&entry_service(id), KEYRING_ACCOUNT)
            .with_context(|| format!("creating keyring entry for '{}'", entry_service(id)))
    }

    fn backend_get(&self, id: &CredentialId) -> Result<Option<String>> {
        let entry = Self::entry(id)?;
        match entry.get_secret() {
            Ok(bytes) => {
                let value = String::from_utf8(bytes).context(
                    "keyring secret was not valid UTF-8 (unexpected for am's own JSON encoding)",
                )?;
                Ok(Some(value))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(err).context("reading secret from the OS keyring"),
        }
    }

    fn backend_set(&self, id: &CredentialId, value: &str) -> Result<()> {
        let entry = Self::entry(id)?;
        entry.set_secret(value.as_bytes()).with_context(|| {
            format!(
                "storing secret for '{}' in the OS keyring",
                entry_service(id)
            )
        })
    }

    fn backend_delete(&self, id: &CredentialId) -> Result<()> {
        let entry = Self::entry(id)?;
        // Idempotent: a missing entry is not an error.
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err).context("deleting secret from the OS keyring"),
        }
    }
}

impl SecretStore for KeyringSecretStore {
    fn list(&self) -> Result<Vec<CredentialMeta>> {
        // ponytail: list() trusts the index file; a native enumeration is the
        // upgrade path if the index ever drifts from the backend (the
        // `keyring` crate has no cross-platform "enumerate all entries" API).
        let mut metas: Vec<CredentialMeta> = self
            .read_index()?
            .into_iter()
            .map(|k| CredentialMeta {
                id: CredentialId {
                    harness: k.harness,
                    name: k.name,
                },
                engine: "keyring".to_string(),
                captured: Default::default(),
            })
            .collect();
        metas.sort_by(|a, b| {
            (a.id.harness.as_str(), a.id.name.as_str())
                .cmp(&(b.id.harness.as_str(), b.id.name.as_str()))
        });
        Ok(metas)
    }

    fn get(&self, id: &CredentialId) -> Result<Option<Vec<CredentialBlob>>> {
        match self.backend_get(id)? {
            Some(value) => Ok(Some(Self::decode(&value)?)),
            None => Ok(None),
        }
    }

    fn set(&self, id: &CredentialId, blobs: &[CredentialBlob]) -> Result<()> {
        let value = Self::encode(blobs)?;
        self.backend_set(id, &value)?;
        self.index_add(id)
    }

    fn delete(&self, id: &CredentialId) -> Result<()> {
        self.backend_delete(id)?;
        self.index_remove(id)
    }

    fn rename(&self, from: &CredentialId, to_name: &str) -> Result<()> {
        let blobs = self.get(from)?.ok_or_else(|| {
            anyhow::anyhow!("no credential '{}/{}' to rename", from.harness, from.name)
        })?;
        let to = CredentialId {
            harness: from.harness.clone(),
            name: to_name.to_string(),
        };
        self.set(&to, &blobs)?;
        self.delete(from)
    }
}

/// The service name a `(harness, name)` maps to inside the OS store.
/// `// ponytail: assumes harness ids/names contain no ':' — true for all current ids.`
fn entry_service(id: &CredentialId) -> String {
    format!("am:{}:{}", id.harness, id.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(name: &str, bytes: &[u8]) -> CredentialBlob {
        CredentialBlob {
            name: name.to_string(),
            rel_path: PathBuf::from(name),
            bytes: bytes.to_vec(),
        }
    }

    // --- Shared logic (encoding + index), no OS keyring involved ----------

    #[test]
    fn wire_blob_json_round_trips() -> Result<()> {
        let blobs = vec![
            blob(".credentials.json", b"{\"claudeAiOauth\":{}}"),
            blob(".claude.json", b"{\"id\":1}"),
        ];
        let encoded = KeyringSecretStore::encode(&blobs)?;
        // Pure ASCII, no NUL — safe as a single opaque secret string.
        assert!(encoded.is_ascii());
        assert!(!encoded.contains('\0'));
        let decoded = KeyringSecretStore::decode(&encoded)?;
        assert_eq!(decoded, blobs);
        Ok(())
    }

    #[test]
    fn index_add_list_remove_round_trip() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let store = KeyringSecretStore::new(temp.path());
        let id = CredentialId {
            harness: "claude-code".to_string(),
            name: "default".to_string(),
        };

        assert!(store.list()?.is_empty());

        store.index_add(&id)?;
        let metas = store.list()?;
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, id);
        assert_eq!(metas[0].engine, "keyring");

        // Adding twice is a no-op (no duplicate entries).
        store.index_add(&id)?;
        assert_eq!(store.list()?.len(), 1);

        store.index_remove(&id)?;
        assert!(store.list()?.is_empty());

        // Removing an absent key is a harmless no-op.
        store.index_remove(&id)?;
        assert!(store.list()?.is_empty());
        Ok(())
    }

    #[test]
    fn index_two_harnesses_same_name_are_independent_entries() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let store = KeyringSecretStore::new(temp.path());
        let claude = CredentialId {
            harness: "claude-code".to_string(),
            name: "default".to_string(),
        };
        let codex = CredentialId {
            harness: "codex".to_string(),
            name: "default".to_string(),
        };
        store.index_add(&claude)?;
        store.index_add(&codex)?;
        assert_eq!(store.list()?.len(), 2);

        store.index_remove(&claude)?;
        let metas = store.list()?;
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, codex);
        Ok(())
    }

    /// Exercises the key-swap the trait's `rename` performs on the index
    /// (add the new key, drop the old one) without touching the OS keyring —
    /// `rename`'s blob copy itself goes through `backend_get`/`backend_set`,
    /// which do need a real store (see the real-backend test below).
    #[test]
    fn index_rename_swaps_the_key() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let store = KeyringSecretStore::new(temp.path());
        let from = CredentialId {
            harness: "claude-code".to_string(),
            name: "default".to_string(),
        };
        store.index_add(&from)?;

        let to = CredentialId {
            harness: from.harness.clone(),
            name: "personal".to_string(),
        };
        store.index_add(&to)?;
        store.index_remove(&from)?;

        let metas = store.list()?;
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, to);
        Ok(())
    }

    #[test]
    fn missing_index_file_is_an_empty_store() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let store = KeyringSecretStore::new(temp.path());
        assert!(store.list()?.is_empty());
        Ok(())
    }

    #[test]
    fn entry_service_embeds_harness_and_name() {
        let id = CredentialId {
            harness: "claude-code".to_string(),
            name: "work".to_string(),
        };
        assert_eq!(entry_service(&id), "am:claude-code:work");
    }

    // --- Real backend (needs an actual OS credential store) ---------------

    // Exercises the real `keyring` backend end-to-end. Skips cleanly (rather
    // than failing) whenever the OS store isn't reachable — e.g. this crate's
    // sandboxed test environment has no keychain/D-Bus session access, same
    // as `OsSecretStore`'s `security`-CLI self-check skips when `security` is
    // unavailable. Uses a distinctive, PID-suffixed name so a real run never
    // collides with a previously stored credential.
    #[test]
    fn keyring_backend_round_trips_when_a_real_store_is_available() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let store = KeyringSecretStore::new(temp.path());
        let id = CredentialId {
            harness: "am-keyring-selftest".to_string(),
            name: format!("selftest-{}", std::process::id()),
        };

        // Absent before anything is stored (also proves get doesn't create).
        match store.get(&id) {
            Ok(None) => {}
            Ok(Some(_)) => panic!("unexpected pre-existing entry for a PID-unique test id"),
            Err(err) => {
                eprintln!("skipping: OS keyring unavailable in this environment: {err:#}");
                return Ok(());
            }
        }

        if let Err(err) = store.set(&id, &[blob(".credentials.json", b"keyring-secret")]) {
            eprintln!("skipping: OS keyring unavailable in this environment: {err:#}");
            return Ok(());
        }

        let got = store.get(&id)?.expect("stored");
        assert_eq!(got[0].bytes, b"keyring-secret");
        assert_eq!(store.list()?.len(), 1);

        store.rename(&id, "renamed")?;
        assert!(store.get(&id)?.is_none());
        let renamed = CredentialId {
            harness: id.harness.clone(),
            name: "renamed".to_string(),
        };
        assert_eq!(store.get(&renamed)?.unwrap()[0].bytes, b"keyring-secret");

        // delete is idempotent.
        store.delete(&renamed)?;
        store.delete(&renamed)?;
        assert!(store.get(&renamed)?.is_none());
        assert!(store.list()?.is_empty());
        Ok(())
    }
}
