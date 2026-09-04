//! Credential storage: a small, pluggable seam for harness login secrets.
//!
//! `am` already separates credential *references* (env-var names, a helper
//! command, a path to a private home dir — see [`crate::account`]) from
//! secret material. This module is the other half: a place to actually
//! *store* captured login bytes (the files [`crate::harness::seed_login`]
//! copies into a relocated config dir) behind a small trait, so the CLI's
//! plain-files layout and an embedder's encrypted vault can share the same
//! call sites.
//!
//! [`SecretStore`] is deliberately narrow: list/get/set/delete/rename over
//! `(harness, name)` pairs, each holding zero or more [`CredentialBlob`]s (a
//! relative path + bytes, matching [`crate::source::Source::Files`]). Three
//! implementations ship in this crate:
//! - [`MemorySecretStore`] — in-process, for tests and lib-mode callers with
//!   no persistence.
//! - [`FileSecretStore`] — plain files on disk, one directory per
//!   `(name, harness)`. The CLI default.
//! - [`PrivateKeychainStore`] — a single local JSON vault file. Despite the
//!   name this is **not** OS Keychain-backed encryption yet (see the module
//!   doc on [`PrivateKeychainStore`]); it exists as a single-file alternative
//!   to the directory-per-credential layout.
//! - [`OsSecretStore`] — the real, OS-encrypted secure store (`engine = "os"`):
//!   macOS Keychain via a custom keychain file under the config dir, with
//!   Linux (`secret-tool`) and Windows (DPAPI) drafts.
//! - [`KeyringSecretStore`] (optional — `keyring-store` feature; `engine =
//!   "keyring"`) — a sibling to [`OsSecretStore`], storing directly in the OS
//!   system credential store via the [`keyring`] crate instead of shelling
//!   out to `security`. See its module doc for the retrieval-prompt/size
//!   caveats that come with that choice.
//!
//! This is core (no `clap`/terminal/tokio, `--no-default-features` builds
//! it): an embedder can substitute its own [`SecretStore`] (e.g. backed by a
//! real OS keychain or a database) without touching the rest of the crate.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::Result;
use crate::harness::SeedFile;
use crate::source::Source;

mod file;
mod keychain;
#[cfg(feature = "keyring-store")]
mod keyring_store;
mod memory;
mod os;

pub use file::FileSecretStore;
pub use keychain::PrivateKeychainStore;
#[cfg(feature = "keyring-store")]
pub use keyring_store::KeyringSecretStore;
pub use memory::MemorySecretStore;
pub use os::OsSecretStore;

/// Identifies one stored credential: which harness it's for, and a
/// user-chosen name (e.g. `"default"`, `"work"`) distinguishing multiple
/// logins for the same harness.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CredentialId {
    /// Harness id (e.g. `"claude-code"`).
    pub harness: String,
    /// Credential name within that harness (e.g. `"default"`, `"work"`).
    pub name: String,
}

/// One file of a stored credential: a path relative to the credential's
/// root, and its raw bytes. Mirrors [`crate::source::Source::Files`]'s
/// `(PathBuf, Vec<u8>)` pairs so the two convert freely (see
/// [`source_from_blobs`] / [`blobs_from_seed`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialBlob {
    /// The file's own name (last path component), for display/listing.
    pub name: String,
    /// Path relative to the credential's root (may be nested, e.g.
    /// `.claude/.credentials.json`).
    pub rel_path: PathBuf,
    /// Raw file bytes.
    pub bytes: Vec<u8>,
}

/// Non-secret metadata about a stored credential, as surfaced by
/// [`SecretStore::list`].
#[derive(Debug, Clone)]
pub struct CredentialMeta {
    /// Which credential this describes.
    pub id: CredentialId,
    /// Which [`SecretStore`] impl produced this entry (`"memory"`, `"files"`,
    /// `"keychain"`), for display purposes.
    pub engine: String,
    // ponytail: captured meta not persisted; add a sidecar if `am account ls`
    // needs it — v1 always returns an empty map here.
    /// Non-secret captured metadata (auth type, plan tier, …). Always empty
    /// in v1 — not yet persisted by any [`SecretStore`] impl.
    pub captured: BTreeMap<String, String>,
}

/// The validity of a stored credential, as computed by [`credential_validity`]
/// from any embedded expiry field. Harness-agnostic (it just looks for a
/// numeric `*expire*` key anywhere in the parsed JSON).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Validity {
    /// An expiry was found and is still in the future (or no expiry field was
    /// found but blobs are present — see [`credential_validity`], which uses
    /// [`Validity::Unknown`] for that case, so `expires_at_ms` here is always
    /// `Some`). Kept as `Option` to leave room for future "valid, no expiry".
    Valid {
        /// Epoch-millis expiry, if one was found.
        expires_at_ms: Option<i64>,
    },
    /// An expiry was found and is at/before `now_ms`.
    Expired {
        /// Epoch-millis expiry that has passed.
        expires_at_ms: i64,
    },
    /// Blobs are present but carry no recognizable expiry field.
    Unknown,
    /// No blobs are stored for this credential.
    Empty,
}

/// Compute a stored credential's [`Validity`] at `now_ms` (epoch millis).
///
/// Harness-agnostic but Claude-aware: for each blob, parse the bytes as JSON
/// and recursively search for a numeric field whose key case-insensitively
/// contains `"expire"` (covers Claude's `claudeAiOauth.expiresAt`, which is
/// epoch **millis**). Each value is normalized — numbers below `10^12` are
/// treated as **seconds** and scaled to millis — and the **maximum** expiry
/// across all blobs wins. Pure (takes `now_ms`) so it's unit-testable without
/// a clock. Returns [`Validity::Empty`] for no blobs, [`Validity::Unknown`]
/// for blobs with no expiry field, else `Valid`/`Expired`.
pub fn credential_validity(blobs: &[CredentialBlob], now_ms: i64) -> Validity {
    if blobs.is_empty() {
        return Validity::Empty;
    }
    let mut max_expiry: Option<i64> = None;
    for blob in blobs {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&blob.bytes)
            && let Some(ms) = max_expiry_ms(&v)
        {
            max_expiry = Some(max_expiry.map_or(ms, |cur| cur.max(ms)));
        }
    }
    match max_expiry {
        Some(ms) if ms > now_ms => Validity::Valid {
            expires_at_ms: Some(ms),
        },
        Some(ms) => Validity::Expired { expires_at_ms: ms },
        None => Validity::Unknown,
    }
}

/// Recursively find the maximum numeric `*expire*` value in `v`, normalized to
/// epoch millis (values below `10^12` are treated as seconds and ×1000).
fn max_expiry_ms(v: &serde_json::Value) -> Option<i64> {
    fn normalize(n: i64) -> i64 {
        if n < 1_000_000_000_000 {
            n.saturating_mul(1000)
        } else {
            n
        }
    }
    match v {
        serde_json::Value::Object(map) => {
            let mut best: Option<i64> = None;
            for (k, val) in map {
                if k.to_lowercase().contains("expire")
                    && let Some(n) = val.as_i64().or_else(|| val.as_f64().map(|f| f as i64))
                {
                    let ms = normalize(n);
                    best = Some(best.map_or(ms, |cur| cur.max(ms)));
                }
                if let Some(ms) = max_expiry_ms(val) {
                    best = Some(best.map_or(ms, |cur| cur.max(ms)));
                }
            }
            best
        }
        serde_json::Value::Array(items) => {
            let mut best: Option<i64> = None;
            for item in items {
                if let Some(ms) = max_expiry_ms(item) {
                    best = Some(best.map_or(ms, |cur| cur.max(ms)));
                }
            }
            best
        }
        _ => None,
    }
}

/// A store of harness login secrets, keyed by [`CredentialId`].
///
/// Every method is synchronous and local — no network I/O is implied by the
/// trait itself (an embedder backing this with a remote vault is free to
/// block internally). All three shipped impls ([`MemorySecretStore`],
/// [`FileSecretStore`], [`PrivateKeychainStore`]) are `Send + Sync` so a
/// `Box<dyn SecretStore>` can be shared behind an `Arc` if needed.
pub trait SecretStore: Send + Sync {
    /// List every stored credential's metadata. Order is unspecified beyond
    /// what each impl documents.
    fn list(&self) -> Result<Vec<CredentialMeta>>;
    /// Fetch a credential's blobs. `Ok(None)` means no such credential is
    /// stored (distinct from a stored-but-empty credential, which returns
    /// `Ok(Some(vec![]))`).
    fn get(&self, id: &CredentialId) -> Result<Option<Vec<CredentialBlob>>>;
    /// Store (creating or overwriting) a credential's blobs.
    fn set(&self, id: &CredentialId, blobs: &[CredentialBlob]) -> Result<()>;
    /// Remove a credential. Idempotent — succeeds even if `id` isn't stored.
    fn delete(&self, id: &CredentialId) -> Result<()>;
    /// Rename a credential's `name` within the SAME harness (`from.harness`
    /// is fixed; only the name component changes). Errors if `from` isn't
    /// stored.
    fn rename(&self, from: &CredentialId, to_name: &str) -> Result<()>;
}

/// Build blobs from a login [`Source`] per a harness's
/// [`crate::harness::ConfigAnchor::login_seed`] — the credential-store
/// counterpart of [`crate::harness::seed_login`] (which writes such files
/// into a relocated run dir instead of a [`SecretStore`]).
///
/// For each [`SeedFile`], reads `source` at `seed.src`; entries whose source
/// file is absent are silently skipped (a partially-captured login still
/// yields the blobs it has). Each resulting blob's `name` is the last path
/// component of `seed.src` (lossy string); `rel_path` is `seed.src` itself
/// (so a later [`source_from_blobs`] round-trips it back to the same
/// source-relative layout the seed files describe).
pub fn blobs_from_seed(source: &Source, seed: &[SeedFile]) -> Result<Vec<CredentialBlob>> {
    let mut blobs = Vec::new();
    for file in seed {
        let Some(bytes) = source.read(&file.src)? else {
            continue;
        };
        let name = file
            .src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.src.to_string_lossy().into_owned());
        blobs.push(CredentialBlob {
            name,
            rel_path: file.src.clone(),
            bytes,
        });
    }
    Ok(blobs)
}

/// Convert stored blobs back into a [`Source::Files`] (each blob's
/// `rel_path` → `bytes`), for handing to [`crate::harness::seed_login`] or
/// [`Source::materialize`](crate::source::Source::materialize).
pub fn source_from_blobs(blobs: &[CredentialBlob]) -> Source {
    Source::Files(
        blobs
            .iter()
            .map(|b| (b.rel_path.clone(), b.bytes.clone()))
            .collect(),
    )
}

/// Resolve which [`SecretStore`] engine to build from (highest precedence
/// first): the `AM_CREDENTIALS_ENGINE` env var (if non-empty), else
/// `settings.credentials.engine`, else the auto default (`"files"`).
///
/// Callers that need to branch on *where* secrets land (e.g. `am account
/// import` avoiding plaintext files under a secure engine) can read this.
pub fn resolve_engine(settings: &crate::settings::Settings) -> String {
    std::env::var("AM_CREDENTIALS_ENGINE")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| settings.credentials.engine.clone())
        // ponytail: default to plain files; keychain (today just a single
        // local JSON vault, not real OS-keychain encryption) stays opt-in
        // until a real DEK/encryption layer lands.
        .unwrap_or_else(|| "files".to_string())
}

/// Resolve the directory the `"keychain"`/`"os"` engines root at:
/// `settings.credentials.keychain_dir`, else `AM_KEYCHAIN`, else
/// `<config dir>/keychain`.
fn keychain_dir(settings: &crate::settings::Settings) -> Result<PathBuf> {
    settings
        .credentials
        .keychain_dir
        .clone()
        .map(PathBuf::from)
        .or_else(|| std::env::var("AM_KEYCHAIN").ok().map(PathBuf::from))
        .or_else(|| crate::settings::default_config_dir().map(|d| d.join("keychain")))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "could not resolve a keychain directory for the credentials store \
                 (set [credentials].keychain_dir, AM_KEYCHAIN, or AM_CREDENTIALS_ENGINE)"
            )
        })
}

/// Build the [`SecretStore`] the CLI (or an embedder) should use, from
/// [`crate::settings::Settings`] and environment overrides.
///
/// Engine resolution: `AM_CREDENTIALS_ENGINE` env var, else
/// `settings.credentials.engine`, else `"files"`.
///
/// - `"files"` roots at `settings.credentials.files_root`, else
///   [`crate::account::resolve_accounts_root`], else an error.
/// - `"keychain"` roots at `keychain_dir` (a plaintext local JSON vault —
///   not OS-encrypted; see [`PrivateKeychainStore`]).
/// - `"os"` (the real, OS-encrypted secure store — macOS Keychain, with
///   Linux/Windows drafts) roots at `keychain_dir` too.
/// - `"keyring"` (optional — `keyring-store` feature; see
///   [`KeyringSecretStore`]) roots at `keychain_dir` too. Selecting it in a
///   build without the feature is an error.
/// - any other string is an error naming the unknown engine.
pub fn build_secret_store(settings: &crate::settings::Settings) -> Result<Box<dyn SecretStore>> {
    let engine = resolve_engine(settings);
    match engine.as_str() {
        "files" => {
            let root = settings
                .credentials
                .files_root
                .clone()
                .map(PathBuf::from)
                .or_else(|| crate::account::resolve_accounts_root(None))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "could not resolve a files root for the credentials store \
                         (set [credentials].files_root, AM_ACCOUNTS, or AM_CREDENTIALS_ENGINE)"
                    )
                })?;
            Ok(Box::new(FileSecretStore::new(root)))
        }
        "keychain" => Ok(Box::new(PrivateKeychainStore::new(keychain_dir(settings)?))),
        "os" => Ok(Box::new(OsSecretStore::new(keychain_dir(settings)?))),
        #[cfg(feature = "keyring-store")]
        "keyring" => Ok(Box::new(KeyringSecretStore::new(keychain_dir(settings)?))),
        #[cfg(not(feature = "keyring-store"))]
        "keyring" => anyhow::bail!(
            "the \"keyring\" credentials engine requires agent-manager to be built with the \
             \"keyring-store\" feature"
        ),
        other => anyhow::bail!(
            "unknown credentials engine '{other}' (expected \"files\", \"keychain\", \"os\", or \"keyring\")"
        ),
    }
}

/// An [`crate::account::AccountStore`] whose login *bodies* come from a
/// [`SecretStore`], keyed by `(harness, name)`, while the account *index*
/// (list / lookup / login capture) stays delegated to an inner store.
///
/// This is the wiring that makes credentials **harness-scoped**: the harness
/// is fixed for the duration of one run (an `am <harness>` invocation), so it's
/// captured here at construction time and combined with the account id (the
/// credential *name*) to form a [`CredentialId`]. [`login_source`] tries the
/// secret store first and falls back to the inner store's own login source —
/// so a legacy `accounts/<name>/` home (from `am account login` or the old
/// Keychain import) still resolves for names not yet migrated into the
/// [`SecretStore`]. See `_docs/inbox/os-secret-store.md` §6, §10.
///
/// [`login_source`]: SecretBackedAccountStore::login_source
pub struct SecretBackedAccountStore {
    inner: Box<dyn crate::account::AccountStore>,
    secrets: Box<dyn SecretStore>,
    harness: String,
}

impl SecretBackedAccountStore {
    /// Wrap `inner` (the index + legacy login source) so login bodies for
    /// `harness` are served from `secrets`.
    pub fn new(
        inner: Box<dyn crate::account::AccountStore>,
        secrets: Box<dyn SecretStore>,
        harness: impl Into<String>,
    ) -> Self {
        SecretBackedAccountStore {
            inner,
            secrets,
            harness: harness.into(),
        }
    }
}

impl crate::account::AccountStore for SecretBackedAccountStore {
    fn accounts(&self) -> Result<Vec<crate::account::Account>> {
        self.inner.accounts()
    }

    fn account(&self, id: &str) -> Result<Option<crate::account::Account>> {
        self.inner.account(id)
    }

    fn login_source(&self, id: &str) -> Result<Option<Source>> {
        let cid = CredentialId {
            harness: self.harness.clone(),
            name: id.to_string(),
        };
        if let Some(blobs) = self.secrets.get(&cid)?
            && !blobs.is_empty()
        {
            return Ok(Some(source_from_blobs(&blobs)));
        }
        // Not in the secret store — fall back to the legacy on-disk home.
        self.inner.login_source(id)
    }

    fn login_home(&self, id: &str) -> Result<PathBuf> {
        self.inner.login_home(id)
    }

    fn capture_login(&self, id: &str, from: &std::path::Path, files: &[PathBuf]) -> Result<()> {
        self.inner.capture_login(id, from, files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::SeedFile;

    fn blob(bytes: &[u8]) -> CredentialBlob {
        CredentialBlob {
            name: "x".to_string(),
            rel_path: PathBuf::from(".claude/.credentials.json"),
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn credential_validity_claude_future_expiry_is_valid() {
        // Claude shape: expiresAt in epoch MILLIS, far in the future.
        let future = 5_000_000_000_000i64; // year ~2128
        let bytes = format!("{{\"claudeAiOauth\":{{\"expiresAt\":{future}}}}}");
        let v = credential_validity(&[blob(bytes.as_bytes())], 1_000_000_000_000);
        assert_eq!(
            v,
            Validity::Valid {
                expires_at_ms: Some(future)
            }
        );
    }

    #[test]
    fn credential_validity_past_expiry_is_expired() {
        let past = 1_000_000_000_000i64;
        let bytes = format!("{{\"claudeAiOauth\":{{\"expiresAt\":{past}}}}}");
        let v = credential_validity(&[blob(bytes.as_bytes())], 2_000_000_000_000);
        assert_eq!(
            v,
            Validity::Expired {
                expires_at_ms: past
            }
        );
    }

    #[test]
    fn credential_validity_no_expiry_field_is_unknown() {
        let v = credential_validity(&[blob(b"{\"apiKey\":\"sk-1\"}")], 1_000);
        assert_eq!(v, Validity::Unknown);
    }

    #[test]
    fn credential_validity_empty_blobs_is_empty() {
        assert_eq!(credential_validity(&[], 1_000), Validity::Empty);
    }

    #[test]
    fn credential_validity_normalizes_seconds_to_millis() {
        // A bare epoch-SECONDS expiry (< 10^12) must be scaled by 1000 so it
        // compares correctly against a millis `now`. 2_000_000_000s = year
        // 2033, well ahead of a `now` of 1_500_000_000_000ms (~2017).
        let secs = 2_000_000_000i64;
        let bytes = format!("{{\"expires_at\":{secs}}}");
        let v = credential_validity(&[blob(bytes.as_bytes())], 1_500_000_000_000);
        assert_eq!(
            v,
            Validity::Valid {
                expires_at_ms: Some(secs * 1000)
            }
        );
    }

    #[test]
    fn credential_validity_takes_max_expiry_across_blobs() {
        let near = 1_600_000_000_000i64;
        let far = 4_000_000_000_000i64;
        let b1 = format!("{{\"expiresAt\":{near}}}");
        let b2 = format!("{{\"expiresAt\":{far}}}");
        let v = credential_validity(
            &[blob(b1.as_bytes()), blob(b2.as_bytes())],
            1_000_000_000_000,
        );
        assert_eq!(
            v,
            Validity::Valid {
                expires_at_ms: Some(far)
            }
        );
    }

    #[test]
    fn blobs_from_seed_round_trips_claude_style_paths() -> Result<()> {
        let source = Source::Files(vec![
            (
                PathBuf::from(".claude/.credentials.json"),
                b"{\"claudeAiOauth\":{}}".to_vec(),
            ),
            (PathBuf::from(".claude.json"), b"{\"id\":1}".to_vec()),
        ]);
        let seed = vec![
            SeedFile::new(".claude/.credentials.json", ".credentials.json"),
            SeedFile::new(".claude.json", ".claude.json"),
        ];

        let blobs = blobs_from_seed(&source, &seed)?;
        assert_eq!(blobs.len(), 2);

        let creds = blobs
            .iter()
            .find(|b| b.rel_path == *".claude/.credentials.json")
            .expect("credentials blob present");
        assert_eq!(creds.name, ".credentials.json");
        assert_eq!(creds.bytes, b"{\"claudeAiOauth\":{}}");

        let identity = blobs
            .iter()
            .find(|b| b.rel_path == *".claude.json")
            .expect("identity blob present");
        assert_eq!(identity.name, ".claude.json");

        // source_from_blobs round-trips back to a readable Source.
        let rebuilt = source_from_blobs(&blobs);
        assert_eq!(
            rebuilt
                .read(&PathBuf::from(".claude/.credentials.json"))?
                .as_deref(),
            Some(&b"{\"claudeAiOauth\":{}}"[..])
        );
        assert_eq!(
            rebuilt.read(&PathBuf::from(".claude.json"))?.as_deref(),
            Some(&b"{\"id\":1}"[..])
        );
        Ok(())
    }

    #[test]
    fn blobs_from_seed_skips_absent_sources() -> Result<()> {
        let source = Source::Files(vec![(
            PathBuf::from(".claude/.credentials.json"),
            b"present".to_vec(),
        )]);
        let seed = vec![
            SeedFile::new(".claude/.credentials.json", ".credentials.json"),
            SeedFile::new(".claude.json", ".claude.json"),
        ];
        let blobs = blobs_from_seed(&source, &seed)?;
        assert_eq!(blobs.len(), 1);
        assert_eq!(blobs[0].name, ".credentials.json");
        Ok(())
    }

    #[test]
    fn build_secret_store_files_engine_via_settings() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let mut settings = crate::settings::Settings::default();
        settings.credentials.engine = Some("files".to_string());
        settings.credentials.files_root = Some(temp.path().to_string_lossy().into_owned());

        let store = build_secret_store(&settings)?;
        let id = CredentialId {
            harness: "claude-code".to_string(),
            name: "default".to_string(),
        };
        let blobs = vec![CredentialBlob {
            name: "x".to_string(),
            rel_path: PathBuf::from("x"),
            bytes: b"hi".to_vec(),
        }];
        store.set(&id, &blobs)?;
        let got = store.get(&id)?.expect("stored");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].bytes, b"hi");
        Ok(())
    }

    #[test]
    fn build_secret_store_unknown_engine_errors() {
        let mut settings = crate::settings::Settings::default();
        settings.credentials.engine = Some("bogus".to_string());
        // `.unwrap_err()` needs `T: Debug`, but `Box<dyn SecretStore>` isn't
        // `Debug`; match the `Err` arm directly instead.
        match build_secret_store(&settings) {
            Ok(_) => panic!("expected an error"),
            Err(err) => assert!(err.to_string().contains("bogus"), "{err}"),
        }
    }

    #[test]
    fn secret_backed_store_serves_login_from_secrets_and_falls_back() -> Result<()> {
        use crate::account::{AccountStore, EmptyAccountStore};

        let secrets = MemorySecretStore::new();
        secrets.set(
            &CredentialId {
                harness: "claude-code".to_string(),
                name: "default".to_string(),
            },
            &[CredentialBlob {
                name: ".credentials.json".to_string(),
                rel_path: PathBuf::from(".claude/.credentials.json"),
                bytes: b"tok".to_vec(),
            }],
        )?;

        let store = SecretBackedAccountStore::new(
            Box::new(EmptyAccountStore),
            Box::new(secrets),
            "claude-code",
        );

        // Hit: served from the secret store as Source::Files.
        let src = store.login_source("default")?.expect("login source");
        assert_eq!(
            src.read(&PathBuf::from(".claude/.credentials.json"))?
                .as_deref(),
            Some(&b"tok"[..])
        );

        // Miss: falls through to the inner store (empty → None).
        assert!(store.login_source("nonexistent")?.is_none());
        Ok(())
    }

    #[test]
    fn settings_load_parses_credentials_engine() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let path = temp.path().join("am.toml");
        std::fs::write(&path, "[credentials]\nengine = \"keychain\"\n")?;
        let settings = crate::settings::load(&path)?;
        assert_eq!(settings.credentials.engine.as_deref(), Some("keychain"));
        Ok(())
    }
}
