//! Plain-files [`SecretStore`]: one directory per `(name, harness)` pair on
//! disk, mirroring the shape of [`crate::account::FsAccountStore`]'s
//! per-account homes. The CLI's default engine (see
//! [`super::build_secret_store`]).

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::Result;

use super::{CredentialBlob, CredentialId, CredentialMeta, SecretStore};

/// A [`SecretStore`] rooted at a directory, laid out
/// `<root>/<name>/<harness>/<rel_path>` — e.g.
/// `<root>/default/claude-code/.claude/.credentials.json`.
#[derive(Debug, Clone)]
pub struct FileSecretStore {
    root: PathBuf,
}

impl FileSecretStore {
    /// Create a store rooted at `root` (created lazily on first `set`).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        FileSecretStore { root: root.into() }
    }

    /// The on-disk directory for one credential: `<root>/<name>/<harness>`.
    fn entry_dir(&self, id: &CredentialId) -> PathBuf {
        self.root.join(&id.name).join(&id.harness)
    }

    /// Write `bytes` to `dir/rel`, creating parent dirs, mode 0600 on unix.
    fn write_file(dir: &Path, rel: &Path, bytes: &[u8]) -> Result<()> {
        let target = dir.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&target, bytes).with_context(|| format!("writing {}", target.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("chmod 600 {}", target.display()))?;
        }
        Ok(())
    }
}

impl SecretStore for FileSecretStore {
    fn list(&self) -> Result<Vec<CredentialMeta>> {
        let mut metas = Vec::new();
        if !self.root.is_dir() {
            return Ok(metas);
        }
        for name_entry in std::fs::read_dir(&self.root)
            .with_context(|| format!("reading directory {}", self.root.display()))?
        {
            let name_entry = name_entry?;
            if !name_entry.file_type()?.is_dir() {
                continue;
            }
            let name = name_entry.file_name().to_string_lossy().into_owned();
            let name_dir = name_entry.path();
            for harness_entry in std::fs::read_dir(&name_dir)
                .with_context(|| format!("reading directory {}", name_dir.display()))?
            {
                let harness_entry = harness_entry?;
                if !harness_entry.file_type()?.is_dir() {
                    continue;
                }
                let harness_dir = harness_entry.path();
                let has_file = walkdir::WalkDir::new(&harness_dir)
                    .min_depth(1)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .any(|e| e.file_type().is_file());
                if !has_file {
                    continue;
                }
                let harness = harness_entry.file_name().to_string_lossy().into_owned();
                metas.push(CredentialMeta {
                    id: CredentialId {
                        harness,
                        name: name.clone(),
                    },
                    engine: "files".to_string(),
                    // ponytail: captured meta not persisted; add a sidecar if
                    // `am account ls` needs it.
                    captured: Default::default(),
                });
            }
        }
        metas.sort_by(|a, b| {
            (a.id.harness.as_str(), a.id.name.as_str())
                .cmp(&(b.id.harness.as_str(), b.id.name.as_str()))
        });
        Ok(metas)
    }

    fn get(&self, id: &CredentialId) -> Result<Option<Vec<CredentialBlob>>> {
        let dir = self.entry_dir(id);
        if !dir.is_dir() {
            return Ok(None);
        }
        let mut blobs = Vec::new();
        for entry in walkdir::WalkDir::new(&dir).min_depth(1) {
            let entry = entry.with_context(|| format!("walking {}", dir.display()))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let rel_path = entry
                .path()
                .strip_prefix(&dir)
                .with_context(|| format!("stripping prefix of {}", entry.path().display()))?
                .to_path_buf();
            let bytes = std::fs::read(entry.path())
                .with_context(|| format!("reading {}", entry.path().display()))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            blobs.push(CredentialBlob {
                name,
                rel_path,
                bytes,
            });
        }
        if blobs.is_empty() {
            return Ok(None);
        }
        blobs.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        Ok(Some(blobs))
    }

    fn set(&self, id: &CredentialId, blobs: &[CredentialBlob]) -> Result<()> {
        let dir = self.entry_dir(id);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        for blob in blobs {
            Self::write_file(&dir, &blob.rel_path, &blob.bytes)?;
        }
        Ok(())
    }

    fn delete(&self, id: &CredentialId) -> Result<()> {
        let dir = self.entry_dir(id);
        if dir.is_dir() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        }
        Ok(())
    }

    fn rename(&self, from: &CredentialId, to_name: &str) -> Result<()> {
        let from_dir = self.entry_dir(from);
        if !from_dir.is_dir() {
            anyhow::bail!("no credential '{}/{}' to rename", from.harness, from.name);
        }
        let to_name_dir = self.root.join(to_name);
        std::fs::create_dir_all(&to_name_dir)
            .with_context(|| format!("creating {}", to_name_dir.display()))?;
        let to_dir = to_name_dir.join(&from.harness);
        std::fs::rename(&from_dir, &to_dir)
            .with_context(|| format!("renaming {} -> {}", from_dir.display(), to_dir.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(rel: &str, bytes: &[u8]) -> CredentialBlob {
        CredentialBlob {
            name: Path::new(rel)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            rel_path: PathBuf::from(rel),
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn two_harnesses_same_name_are_independent() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let store = FileSecretStore::new(temp.path());

        let claude = CredentialId {
            harness: "claude-code".to_string(),
            name: "default".to_string(),
        };
        let codex = CredentialId {
            harness: "codex".to_string(),
            name: "default".to_string(),
        };

        store.set(&claude, &[blob("a.json", b"claude-bytes")])?;
        store.set(&codex, &[blob("b.json", b"codex-bytes")])?;

        let claude_got = store.get(&claude)?.expect("claude present");
        assert_eq!(claude_got.len(), 1);
        assert_eq!(claude_got[0].bytes, b"claude-bytes");

        let codex_got = store.get(&codex)?.expect("codex present");
        assert_eq!(codex_got.len(), 1);
        assert_eq!(codex_got[0].bytes, b"codex-bytes");

        store.delete(&claude)?;
        assert!(store.get(&claude)?.is_none());
        // codex untouched.
        assert!(store.get(&codex)?.is_some());
        Ok(())
    }

    #[test]
    fn nested_rel_path_survives_round_trip() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let store = FileSecretStore::new(temp.path());
        let id = CredentialId {
            harness: "claude-code".to_string(),
            name: "default".to_string(),
        };
        store.set(&id, &[blob(".claude/.credentials.json", b"{\"a\":1}")])?;

        let got = store.get(&id)?.expect("present");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].rel_path, PathBuf::from(".claude/.credentials.json"));
        assert_eq!(got[0].name, ".credentials.json");
        assert_eq!(got[0].bytes, b"{\"a\":1}");
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn files_are_written_0600() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::TempDir::new()?;
        let store = FileSecretStore::new(temp.path());
        let id = CredentialId {
            harness: "claude-code".to_string(),
            name: "default".to_string(),
        };
        store.set(&id, &[blob("secret.json", b"tok")])?;

        let path = temp
            .path()
            .join("default")
            .join("claude-code")
            .join("secret.json");
        let mode = std::fs::metadata(&path)?.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "expected 0600, got {mode:o}");
        Ok(())
    }

    #[test]
    fn rename_moves_the_directory() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let store = FileSecretStore::new(temp.path());
        let from = CredentialId {
            harness: "claude-code".to_string(),
            name: "default".to_string(),
        };
        store.set(&from, &[blob("a.json", b"x")])?;

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
    fn rename_of_missing_errors() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = FileSecretStore::new(temp.path());
        let from = CredentialId {
            harness: "claude-code".to_string(),
            name: "ghost".to_string(),
        };
        assert!(store.rename(&from, "renamed").is_err());
    }

    #[test]
    fn list_reports_only_non_empty_entries() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let store = FileSecretStore::new(temp.path());
        let id = CredentialId {
            harness: "claude-code".to_string(),
            name: "default".to_string(),
        };
        store.set(&id, &[blob("a.json", b"x")])?;

        let metas = store.list()?;
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, id);
        assert_eq!(metas[0].engine, "files");
        Ok(())
    }
}
