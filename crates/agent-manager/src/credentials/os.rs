//! Native OS secret-store [`SecretStore`] (`engine = "os"`).
//!
//! Unlike [`super::PrivateKeychainStore`] (a plaintext local JSON vault), this
//! stores credentials in the platform's real, encrypted secret service via its
//! native CLI — no new dependencies, no `unsafe` (subprocess only), same
//! mechanism [`crate::account::read_claude_keychain_credentials`] already uses
//! to *read* Claude's login keychain.
//!
//! One entry per [`CredentialId`], whose value is the JSON of its blobs (the
//! same `{name, rel_path, bytes}` shape [`super::PrivateKeychainStore`]
//! serializes). A small non-secret index file (`<dir>/index.json`, just the
//! `(harness, name)` keys) backs [`SecretStore::list`].
//! `list`/`rename`/blob-encoding are OS-agnostic; only raw get/set/delete of one
//! string per key is per-OS.
//!
//! Provider by target OS:
//! - **macOS (real):** a custom keychain file `<dir>/am.keychain-db`, created and
//!   accessed with the `security` CLI. Its unlock password is generated once and
//!   stored in the user's **login keychain**, so nothing sensitive sits in
//!   plaintext on disk and the encrypted keychain file still lives under
//!   `~/.config/agent-manager` (isol8-relocatable).
//! - **Linux (draft, untested):** Secret Service via `secret-tool`. The secret
//!   service is a system daemon, so there is no config-dir file here.
//! - **Windows (draft, untested):** a per-user DPAPI-encrypted file in the config
//!   dir via PowerShell, keeping the file-in-config-dir property.

use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::Result;

use super::{CredentialBlob, CredentialId, CredentialMeta, SecretStore};

/// Service name under which the macOS keychain's unlock password is kept in the
/// user's login keychain (one entry, account = the vault keychain's path).
#[cfg(target_os = "macos")]
const MAC_VAULT_PW_SERVICE: &str = "agent-manager-vault";

/// One blob as serialized into an entry's value. Mirrors the plaintext vault's
/// shape; serde's default `Vec<u8>` encoding (a JSON array of numbers) keeps the
/// value pure ASCII with no NUL — safe to pass as one argv value.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// A [`SecretStore`] backed by the platform's native secret service.
#[derive(Debug, Clone)]
pub struct OsSecretStore {
    /// Directory holding the OS store's files (keychain file on macOS, index
    /// everywhere).
    dir: PathBuf,
    /// Test-only override so the macOS self-check can use an explicit password +
    /// throwaway keychain instead of touching the real login keychain.
    #[cfg(target_os = "macos")]
    test_password: Option<String>,
}

impl OsSecretStore {
    /// Create a store rooted at `dir` (e.g. `<config>/keychain`).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        OsSecretStore {
            dir: dir.into(),
            #[cfg(target_os = "macos")]
            test_password: None,
        }
    }

    /// Encode a credential's blobs into one entry value (compact JSON).
    fn encode(blobs: &[CredentialBlob]) -> Result<String> {
        let wire: Vec<WireBlob> = blobs.iter().map(WireBlob::from).collect();
        serde_json::to_string(&wire).context("serializing credential blobs for the OS store")
    }

    /// Decode an entry value back into blobs.
    fn decode(value: &str) -> Result<Vec<CredentialBlob>> {
        let wire: Vec<WireBlob> =
            serde_json::from_str(value).context("parsing credential blobs from the OS store")?;
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
        let body = serde_json::to_string_pretty(keys).context("serializing OS store index")?;
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
}

impl SecretStore for OsSecretStore {
    fn list(&self) -> Result<Vec<CredentialMeta>> {
        // ponytail: list() trusts the index file; a native enumeration is the
        // upgrade path if the index ever drifts from the backend.
        let mut metas: Vec<CredentialMeta> = self
            .read_index()?
            .into_iter()
            .map(|k| CredentialMeta {
                id: CredentialId {
                    harness: k.harness,
                    name: k.name,
                },
                engine: "os".to_string(),
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

/// The service name a `(harness, name)` maps to inside the native store.
/// `// ponytail: assumes harness ids/names contain no ':' — true for all current ids.`
fn entry_service(id: &CredentialId) -> String {
    format!("am:{}:{}", id.harness, id.name)
}

// ---------------------------------------------------------------------------
// macOS provider (real): custom keychain file via the `security` CLI.
// ---------------------------------------------------------------------------
#[cfg(target_os = "macos")]
impl OsSecretStore {
    /// Test-only constructor: an explicit unlock password + throwaway keychain
    /// dir, so the self-check never reads or writes the real login keychain.
    #[cfg(test)]
    fn with_test_password(dir: impl Into<PathBuf>, password: impl Into<String>) -> Self {
        OsSecretStore {
            dir: dir.into(),
            test_password: Some(password.into()),
        }
    }

    fn keychain_path(&self) -> PathBuf {
        self.dir.join("am.keychain-db")
    }

    /// A 48-hex-char random password from `/dev/urandom` (no dependency).
    fn random_password() -> Result<String> {
        use std::io::Read;
        let mut buf = [0u8; 24];
        std::fs::File::open("/dev/urandom")
            .context("opening /dev/urandom")?
            .read_exact(&mut buf)
            .context("reading /dev/urandom")?;
        let mut s = String::with_capacity(48);
        for b in buf {
            s.push_str(&format!("{b:02x}"));
        }
        Ok(s)
    }

    /// The vault keychain's unlock password: the test override, else the entry
    /// kept in the user's login keychain (created on first `set`).
    fn mac_password(&self, create: bool) -> Result<String> {
        if let Some(pw) = &self.test_password {
            return Ok(pw.clone());
        }
        let account = self.keychain_path().to_string_lossy().into_owned();
        let found = std::process::Command::new("security")
            .args(["find-generic-password", "-s", MAC_VAULT_PW_SERVICE, "-a", &account, "-w"])
            .output()
            .context("running `security find-generic-password` for the vault password")?;
        if found.status.success() {
            return Ok(String::from_utf8_lossy(&found.stdout).trim().to_string());
        }
        if !create {
            anyhow::bail!(
                "OS keychain vault password not found in the login keychain (service {MAC_VAULT_PW_SERVICE})"
            );
        }
        let pw = Self::random_password()?;
        let stored = std::process::Command::new("security")
            .args([
                "add-generic-password", "-U", "-s", MAC_VAULT_PW_SERVICE, "-a", &account, "-w", &pw,
            ])
            .status()
            .context("storing the vault password in the login keychain")?;
        anyhow::ensure!(stored.success(), "`security add-generic-password` (vault password) failed");
        Ok(pw)
    }

    /// Ensure the custom keychain exists and is unlocked; return its path + pw.
    /// With `create = false`, returns `None` when the keychain file is absent
    /// (so `get`/`delete` never create it as a side effect).
    fn mac_keychain(&self, create: bool) -> Result<Option<(PathBuf, String)>> {
        let path = self.keychain_path();
        if !path.exists() && !create {
            return Ok(None);
        }
        let pw = self.mac_password(create)?;
        if !path.exists() {
            std::fs::create_dir_all(&self.dir)
                .with_context(|| format!("creating {}", self.dir.display()))?;
            let path_str = path.to_string_lossy().into_owned();
            let created = std::process::Command::new("security")
                .args(["create-keychain", "-p", &pw, &path_str])
                .status()
                .context("running `security create-keychain`")?;
            anyhow::ensure!(created.success(), "`security create-keychain` failed");
            // Don't auto-lock on sleep or after a timeout — this is an
            // unattended store, not an interactive login keychain.
            let _ = std::process::Command::new("security")
                .args(["set-keychain-settings", &path_str])
                .status();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
        }
        let unlocked = std::process::Command::new("security")
            .args(["unlock-keychain", "-p", &pw, &path.to_string_lossy()])
            .status()
            .context("running `security unlock-keychain`")?;
        anyhow::ensure!(unlocked.success(), "`security unlock-keychain` failed");
        Ok(Some((path, pw)))
    }

    fn backend_get(&self, id: &CredentialId) -> Result<Option<String>> {
        let Some((path, _)) = self.mac_keychain(false)? else {
            return Ok(None);
        };
        let out = std::process::Command::new("security")
            .args(["find-generic-password", "-s", &entry_service(id), "-w", &path.to_string_lossy()])
            .output()
            .context("running `security find-generic-password`")?;
        if out.status.success() {
            Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_string()))
        } else {
            // errSecItemNotFound (44) — treat any lookup failure as "absent".
            Ok(None)
        }
    }

    fn backend_set(&self, id: &CredentialId, value: &str) -> Result<()> {
        let (path, _) = self.mac_keychain(true)?.expect("create=true always yields a keychain");
        // ponytail: the value is passed on argv, briefly visible to `ps`; the
        // upgrade path is feeding it via stdin/a temp file when `security`
        // grows the option. Bounded: local processes, small window.
        let status = std::process::Command::new("security")
            .args([
                "add-generic-password", "-U", "-s", &entry_service(id), "-a", "am", "-w", value,
                &path.to_string_lossy(),
            ])
            .status()
            .context("running `security add-generic-password`")?;
        anyhow::ensure!(status.success(), "`security add-generic-password` failed");
        Ok(())
    }

    fn backend_delete(&self, id: &CredentialId) -> Result<()> {
        let Some((path, _)) = self.mac_keychain(false)? else {
            return Ok(());
        };
        // Idempotent: a missing entry (exit 44) is not an error.
        let _ = std::process::Command::new("security")
            .args(["delete-generic-password", "-s", &entry_service(id), &path.to_string_lossy()])
            .status()
            .context("running `security delete-generic-password`")?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Linux provider (DRAFT, untested): Secret Service via `secret-tool`.
// No config-dir file — the secret service is a system daemon.
// ---------------------------------------------------------------------------
#[cfg(target_os = "linux")]
impl OsSecretStore {
    fn backend_get(&self, id: &CredentialId) -> Result<Option<String>> {
        let out = std::process::Command::new("secret-tool")
            .args(["lookup", "service", "agent-manager", "harness", &id.harness, "name", &id.name])
            .output()
            .context("running `secret-tool lookup` (is libsecret installed?)")?;
        if out.status.success() && !out.stdout.is_empty() {
            Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_string()))
        } else {
            Ok(None)
        }
    }

    fn backend_set(&self, id: &CredentialId, value: &str) -> Result<()> {
        use std::io::Write;
        // secret-tool store reads the secret from stdin (no argv leak).
        let mut child = std::process::Command::new("secret-tool")
            .args([
                "store", "--label", &format!("am:{}:{}", id.harness, id.name),
                "service", "agent-manager", "harness", &id.harness, "name", &id.name,
            ])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .context("running `secret-tool store`")?;
        child
            .stdin
            .take()
            .context("secret-tool stdin")?
            .write_all(value.as_bytes())?;
        anyhow::ensure!(child.wait()?.success(), "`secret-tool store` failed");
        Ok(())
    }

    fn backend_delete(&self, id: &CredentialId) -> Result<()> {
        let _ = std::process::Command::new("secret-tool")
            .args(["clear", "service", "agent-manager", "harness", &id.harness, "name", &id.name])
            .status()
            .context("running `secret-tool clear`")?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Windows provider (DRAFT, untested): per-user DPAPI file in the config dir,
// via PowerShell ConvertFrom/ConvertTo-SecureString.
// ---------------------------------------------------------------------------
#[cfg(target_os = "windows")]
impl OsSecretStore {
    fn blob_path(&self, id: &CredentialId) -> PathBuf {
        self.dir.join(format!("{}__{}.dpapi", id.harness, id.name))
    }

    fn backend_get(&self, id: &CredentialId) -> Result<Option<String>> {
        let path = self.blob_path(id);
        if !path.is_file() {
            return Ok(None);
        }
        let script = format!(
            "$s = Get-Content -Raw '{}' | ConvertTo-SecureString; \
             [Runtime.InteropServices.Marshal]::PtrToStringAuto(\
             [Runtime.InteropServices.Marshal]::SecureStringToBSTR($s))",
            path.display()
        );
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .context("running PowerShell to decrypt DPAPI blob")?;
        anyhow::ensure!(out.status.success(), "DPAPI decrypt failed");
        Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_string()))
    }

    fn backend_set(&self, id: &CredentialId, value: &str) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.blob_path(id);
        let script = format!(
            "ConvertTo-SecureString -String $env:AM_SECRET -AsPlainText -Force | \
             ConvertFrom-SecureString | Set-Content '{}'",
            path.display()
        );
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .env("AM_SECRET", value)
            .status()
            .context("running PowerShell to encrypt DPAPI blob")?;
        anyhow::ensure!(status.success(), "DPAPI encrypt failed");
        Ok(())
    }

    fn backend_delete(&self, id: &CredentialId) -> Result<()> {
        let path = self.blob_path(id);
        if path.is_file() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Unsupported platforms: clear error (the user can pick `engine = "files"`).
// ---------------------------------------------------------------------------
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
impl OsSecretStore {
    fn backend_get(&self, _id: &CredentialId) -> Result<Option<String>> {
        anyhow::bail!("the OS credentials engine is not supported on this platform")
    }
    fn backend_set(&self, _id: &CredentialId, _value: &str) -> Result<()> {
        anyhow::bail!("the OS credentials engine is not supported on this platform")
    }
    fn backend_delete(&self, _id: &CredentialId) -> Result<()> {
        anyhow::bail!("the OS credentials engine is not supported on this platform")
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    fn blob(name: &str, bytes: &[u8]) -> CredentialBlob {
        CredentialBlob {
            name: name.to_string(),
            rel_path: PathBuf::from(name),
            bytes: bytes.to_vec(),
        }
    }

    // Exercises the real `security` CLI against a throwaway keychain (explicit
    // password, so the login keychain is never touched). Skips cleanly if the
    // `security` binary can't create a keychain (locked-down CI).
    // ponytail: single self-check, real security calls against a throwaway keychain.
    #[test]
    fn os_store_round_trips_two_harnesses_same_name() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let store = OsSecretStore::with_test_password(temp.path(), "am-test-pw");

        let claude = CredentialId { harness: "claude-code".into(), name: "default".into() };
        let codex = CredentialId { harness: "codex".into(), name: "default".into() };

        // Absent before anything is stored (also proves get doesn't create).
        assert!(store.get(&claude)?.is_none());

        if store.set(&claude, &[blob(".credentials.json", b"claude-secret")]).is_err() {
            eprintln!("skipping: `security create-keychain` unavailable in this environment");
            return Ok(());
        }
        store.set(&codex, &[blob("auth.json", b"codex-secret")])?;

        // Same name, different harness → independent entries.
        assert_eq!(store.get(&claude)?.unwrap()[0].bytes, b"claude-secret");
        assert_eq!(store.get(&codex)?.unwrap()[0].bytes, b"codex-secret");

        // list() sees both via the index.
        assert_eq!(store.list()?.len(), 2);

        // rename within the same harness.
        store.rename(&claude, "personal")?;
        assert!(store.get(&claude)?.is_none());
        let personal = CredentialId { harness: "claude-code".into(), name: "personal".into() };
        assert_eq!(store.get(&personal)?.unwrap()[0].bytes, b"claude-secret");

        // delete is idempotent.
        store.delete(&codex)?;
        store.delete(&codex)?;
        assert!(store.get(&codex)?.is_none());

        Ok(())
    }
}
