//! In-process [`SecretStore`]: no persistence, for tests and lib-mode
//! callers that don't need credentials to survive the process.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::Result;

use super::{CredentialBlob, CredentialId, CredentialMeta, SecretStore};

/// An in-memory [`SecretStore`] backed by a `Mutex<HashMap<..>>`. Nothing is
/// ever written to disk; all state is lost when the store is dropped.
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    entries: Mutex<HashMap<CredentialId, Vec<CredentialBlob>>>,
}

impl MemorySecretStore {
    /// An empty in-memory store.
    pub fn new() -> Self {
        MemorySecretStore::default()
    }
}

impl SecretStore for MemorySecretStore {
    fn list(&self) -> Result<Vec<CredentialMeta>> {
        let entries = self
            .entries
            .lock()
            .expect("MemorySecretStore mutex poisoned");
        let mut metas: Vec<CredentialMeta> = entries
            .keys()
            .map(|id| CredentialMeta {
                id: id.clone(),
                engine: "memory".to_string(),
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
        let entries = self
            .entries
            .lock()
            .expect("MemorySecretStore mutex poisoned");
        Ok(entries.get(id).cloned())
    }

    fn set(&self, id: &CredentialId, blobs: &[CredentialBlob]) -> Result<()> {
        let mut entries = self
            .entries
            .lock()
            .expect("MemorySecretStore mutex poisoned");
        entries.insert(id.clone(), blobs.to_vec());
        Ok(())
    }

    fn delete(&self, id: &CredentialId) -> Result<()> {
        let mut entries = self
            .entries
            .lock()
            .expect("MemorySecretStore mutex poisoned");
        entries.remove(id);
        Ok(())
    }

    fn rename(&self, from: &CredentialId, to_name: &str) -> Result<()> {
        let mut entries = self
            .entries
            .lock()
            .expect("MemorySecretStore mutex poisoned");
        let blobs = entries.remove(from).ok_or_else(|| {
            anyhow::anyhow!("no credential '{}/{}' to rename", from.harness, from.name)
        })?;
        let to = CredentialId {
            harness: from.harness.clone(),
            name: to_name.to_string(),
        };
        entries.insert(to, blobs);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(name: &str, bytes: &[u8]) -> CredentialBlob {
        CredentialBlob {
            name: name.to_string(),
            rel_path: std::path::PathBuf::from(name),
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn set_get_delete_round_trip() -> Result<()> {
        let store = MemorySecretStore::new();
        let id = CredentialId {
            harness: "claude-code".to_string(),
            name: "default".to_string(),
        };
        assert!(store.get(&id)?.is_none());

        store.set(&id, &[blob("a", b"1")])?;
        let got = store.get(&id)?.expect("present after set");
        assert_eq!(got, vec![blob("a", b"1")]);

        store.delete(&id)?;
        assert!(store.get(&id)?.is_none());
        // Deleting again is a no-op, not an error.
        store.delete(&id)?;
        Ok(())
    }

    #[test]
    fn rename_moves_within_a_harness() -> Result<()> {
        let store = MemorySecretStore::new();
        let from = CredentialId {
            harness: "claude-code".to_string(),
            name: "default".to_string(),
        };
        store.set(&from, &[blob("a", b"1")])?;

        store.rename(&from, "personal")?;
        assert!(store.get(&from)?.is_none());

        let to = CredentialId {
            harness: "claude-code".to_string(),
            name: "personal".to_string(),
        };
        assert_eq!(store.get(&to)?.unwrap(), vec![blob("a", b"1")]);
        Ok(())
    }

    #[test]
    fn rename_of_missing_credential_errors() {
        let store = MemorySecretStore::new();
        let from = CredentialId {
            harness: "claude-code".to_string(),
            name: "ghost".to_string(),
        };
        assert!(store.rename(&from, "renamed").is_err());
    }

    #[test]
    fn rename_isolation_across_harnesses() -> Result<()> {
        let store = MemorySecretStore::new();
        let claude_default = CredentialId {
            harness: "claude-code".to_string(),
            name: "default".to_string(),
        };
        let codex_default = CredentialId {
            harness: "codex".to_string(),
            name: "default".to_string(),
        };
        store.set(&claude_default, &[blob("c", b"claude")])?;
        store.set(&codex_default, &[blob("x", b"codex")])?;

        store.rename(&claude_default, "personal")?;

        // codex's "default" is untouched.
        assert_eq!(
            store.get(&codex_default)?.unwrap(),
            vec![blob("x", b"codex")]
        );
        // claude's "default" is gone.
        assert!(store.get(&claude_default)?.is_none());
        // claude's "personal" now holds the moved blobs.
        let claude_personal = CredentialId {
            harness: "claude-code".to_string(),
            name: "personal".to_string(),
        };
        assert_eq!(
            store.get(&claude_personal)?.unwrap(),
            vec![blob("c", b"claude")]
        );
        Ok(())
    }
}
