//! Single-vault-file [`SecretStore`].
//!
//! Despite the name, this is **not** OS-Keychain-backed encryption — it's a
//! single local JSON file (`store.json`) holding every credential, written
//! mode `0600` on unix. It exists as an alternative on-disk shape to
//! [`super::FileSecretStore`]'s directory-per-credential layout (useful when
//! an embedder wants one file to back up/sync instead of a tree). Real
//! encryption (a DEK wrapped by the platform keychain) is future work — see
//! the `ponytail` comment in [`super::build_secret_store`].

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::Result;

use super::{CredentialBlob, CredentialId, CredentialMeta, SecretStore};

/// One file within a vault entry, as serialized to disk. Bytes use serde's
/// default `Vec<u8>` encoding (a JSON array of numbers) — no base64, per the
/// "no new dependencies" constraint on this module.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct VaultBlob {
    name: String,
    rel_path: PathBuf,
    bytes: Vec<u8>,
}

impl From<&CredentialBlob> for VaultBlob {
    fn from(b: &CredentialBlob) -> Self {
        VaultBlob {
            name: b.name.clone(),
            rel_path: b.rel_path.clone(),
            bytes: b.bytes.clone(),
        }
    }
}

impl From<VaultBlob> for CredentialBlob {
    fn from(b: VaultBlob) -> Self {
        CredentialBlob {
            name: b.name,
            rel_path: b.rel_path,
            bytes: b.bytes,
        }
    }
}

/// The on-disk vault file shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Vault {
    version: u32,
    /// Keyed by `"<harness>\u{0}<name>"` (NUL-separated so a `/` in either
    /// component is safe).
    entries: BTreeMap<String, Vec<VaultBlob>>,
}

impl Default for Vault {
    fn default() -> Self {
        Vault {
            version: 1,
            entries: BTreeMap::new(),
        }
    }
}

/// NUL-separated map key for `id`, safe against `/` appearing in either
/// component.
fn entry_key(id: &CredentialId) -> String {
    format!("{}\u{0}{}", id.harness, id.name)
}

/// Parse a map key back into a [`CredentialId`]. `None` if malformed
/// (defensive — every key this store writes is well-formed).
fn parse_entry_key(key: &str) -> Option<CredentialId> {
    let (harness, name) = key.split_once('\u{0}')?;
    Some(CredentialId {
        harness: harness.to_string(),
        name: name.to_string(),
    })
}

/// A [`SecretStore`] backed by a single JSON vault file at `<dir>/store.json`.
#[derive(Debug, Clone)]
pub struct PrivateKeychainStore {
    /// The vault file path (`<dir>/store.json`).
    path: PathBuf,
}

impl PrivateKeychainStore {
    /// Create a store whose vault file is `<dir>/store.json`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        PrivateKeychainStore {
            path: dir.into().join("store.json"),
        }
    }

    /// Load the vault from disk, or an empty one if the file doesn't exist.
    fn load(&self) -> Result<Vault> {
        if !self.path.is_file() {
            return Ok(Vault::default());
        }
        let raw = std::fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", self.path.display()))
    }

    /// Write the vault to disk, creating the parent dir, mode 0600 on unix.
    fn save(&self, vault: &Vault) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let body = serde_json::to_string_pretty(vault)
            .with_context(|| "serializing credentials vault".to_string())?;
        std::fs::write(&self.path, body)
            .with_context(|| format!("writing {}", self.path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("chmod 600 {}", self.path.display()))?;
        }
        Ok(())
    }
}

impl SecretStore for PrivateKeychainStore {
    fn list(&self) -> Result<Vec<CredentialMeta>> {
        let vault = self.load()?;
        let mut metas: Vec<CredentialMeta> = vault
            .entries
            .keys()
            .filter_map(|k| parse_entry_key(k))
            .map(|id| CredentialMeta {
                id,
                engine: "keychain".to_string(),
                // ponytail: captured meta not persisted; add a sidecar if
                // `am account ls` needs it.
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
        let vault = self.load()?;
        Ok(vault
            .entries
            .get(&entry_key(id))
            .map(|blobs| blobs.iter().cloned().map(CredentialBlob::from).collect()))
    }

    fn set(&self, id: &CredentialId, blobs: &[CredentialBlob]) -> Result<()> {
        let mut vault = self.load()?;
        vault
            .entries
            .insert(entry_key(id), blobs.iter().map(VaultBlob::from).collect());
        self.save(&vault)
    }

    fn delete(&self, id: &CredentialId) -> Result<()> {
        let mut vault = self.load()?;
        vault.entries.remove(&entry_key(id));
        self.save(&vault)
    }

    fn rename(&self, from: &CredentialId, to_name: &str) -> Result<()> {
        let mut vault = self.load()?;
        let blobs = vault
            .entries
            .remove(&entry_key(from))
            .ok_or_else(|| anyhow::anyhow!("no credential '{}/{}' to rename", from.harness, from.name))?;
        let to = CredentialId {
            harness: from.harness.clone(),
            name: to_name.to_string(),
        };
        vault.entries.insert(entry_key(&to), blobs);
        self.save(&vault)
    }
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

    #[test]
    fn set_get_delete_round_trip() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let store = PrivateKeychainStore::new(temp.path());
        let id = CredentialId {
            harness: "claude-code".to_string(),
            name: "default".to_string(),
        };
        assert!(store.get(&id)?.is_none());

        store.set(&id, &[blob("a", b"secret")])?;
        let got = store.get(&id)?.expect("present");
        assert_eq!(got[0].bytes, b"secret");

        // A fresh store instance pointed at the same dir reads it back.
        let store2 = PrivateKeychainStore::new(temp.path());
        assert_eq!(store2.get(&id)?.unwrap()[0].bytes, b"secret");

        store.delete(&id)?;
        assert!(store.get(&id)?.is_none());
        Ok(())
    }

    #[test]
    fn two_harnesses_same_name_are_independent() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let store = PrivateKeychainStore::new(temp.path());
        let claude = CredentialId {
            harness: "claude-code".to_string(),
            name: "default".to_string(),
        };
        let codex = CredentialId {
            harness: "codex".to_string(),
            name: "default".to_string(),
        };
        store.set(&claude, &[blob("a", b"claude")])?;
        store.set(&codex, &[blob("b", b"codex")])?;

        assert_eq!(store.get(&claude)?.unwrap()[0].bytes, b"claude");
        assert_eq!(store.get(&codex)?.unwrap()[0].bytes, b"codex");

        store.delete(&claude)?;
        assert!(store.get(&claude)?.is_none());
        assert!(store.get(&codex)?.is_some());
        Ok(())
    }

    #[test]
    fn rename_within_same_harness() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let store = PrivateKeychainStore::new(temp.path());
        let from = CredentialId {
            harness: "claude-code".to_string(),
            name: "default".to_string(),
        };
        store.set(&from, &[blob("a", b"x")])?;
        store.rename(&from, "personal")?;
        assert!(store.get(&from)?.is_none());
        let to = CredentialId {
            harness: "claude-code".to_string(),
            name: "personal".to_string(),
        };
        assert_eq!(store.get(&to)?.unwrap()[0].bytes, b"x");
        Ok(())
    }

    #[test]
    fn missing_vault_file_is_an_empty_store() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let store = PrivateKeychainStore::new(temp.path());
        assert!(store.list()?.is_empty());
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn vault_file_is_written_0600() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::TempDir::new()?;
        let store = PrivateKeychainStore::new(temp.path());
        let id = CredentialId {
            harness: "claude-code".to_string(),
            name: "default".to_string(),
        };
        store.set(&id, &[blob("a", b"x")])?;
        let mode = std::fs::metadata(temp.path().join("store.json"))?
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "expected 0600, got {mode:o}");
        Ok(())
    }
}
