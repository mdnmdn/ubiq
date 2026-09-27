//! Credential storage: a small, pluggable seam for secrets an embedder keeps.
//!
//! `am` separates credential *references* (env-var names, a helper command —
//! see [`crate::account`]) from secret material. This module is the other
//! half: a place to actually *store* secret bytes behind a small trait, so a
//! plain-files layout and an embedder's encrypted vault share the same call
//! sites. A harness login is not one of them: it stays in the harness's own
//! config home and nothing here reads or copies it (`D193`).
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
/// `(PathBuf, Vec<u8>)` pairs.
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
///
/// A key naming a *refresh* expiry is skipped, because it is not the one that
/// decides whether the credential works: Claude's blob carries both
/// `expiresAt` (the access token, hours) and `refreshTokenExpiresAt` (the
/// refresh window, thirty days), and taking the maximum reported a session as
/// good for a month when its access token had hours left. See [`is_expiry`].
///
/// A blob that is [`not usable`](login_is_usable) — every token field an empty
/// string, which is what a harness writes when it signs itself out — carries no
/// login at all, so it contributes nothing and an account holding only such
/// blobs reads as [`Validity::Empty`] rather than as a live session.
pub fn credential_validity(blobs: &[CredentialBlob], now_ms: i64) -> Validity {
    let blobs: Vec<&CredentialBlob> = blobs.iter().filter(|b| login_is_usable(&b.bytes)).collect();
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

/// Whether `key` names the expiry that decides if a credential still works.
///
/// `*expire*`, minus anything that also says `refresh`: an OAuth blob carries
/// two clocks, and the refresh window is the longer one by an order of
/// magnitude. Claude Code's `refreshTokenExpiresAt` is thirty days out while
/// its `expiresAt` is hours, so a maximum that counted both reported a dead
/// session as a month of runway.
fn is_expiry(key: &str) -> bool {
    let key = key.to_lowercase();
    key.contains("expire") && !key.contains("refresh")
}

/// Whether a credential blob carries an actual secret, as opposed to the
/// hollowed-out shell a harness leaves behind when it signs itself out.
///
/// Claude Code rewrites `.credentials.json` in place with `"accessToken": ""`,
/// `"refreshToken": ""` and `"expiresAt": 0` when a refresh fails — the file
/// stays, the account keeps looking logged in, and the surviving
/// `refreshTokenExpiresAt` even dates it a month into the future. Treating
/// that as a login would report a signed-out credential as live, so
/// [`credential_validity`] asks this first.
///
/// Harness-agnostic: any JSON object with `*token*` keys is unusable when
/// **every** one of them is an empty string. Non-JSON bytes, and JSON naming
/// no token at all, are left alone — this is a check for a known-empty
/// credential, not a claim to recognise every shape of a full one.
pub fn login_is_usable(bytes: &[u8]) -> bool {
    fn tokens(v: &serde_json::Value, found: &mut bool, any_set: &mut bool) {
        match v {
            serde_json::Value::Object(map) => {
                for (k, val) in map {
                    if k.to_lowercase().contains("token")
                        && let Some(s) = val.as_str()
                    {
                        *found = true;
                        *any_set |= !s.is_empty();
                    }
                    tokens(val, found, any_set);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    tokens(item, found, any_set);
                }
            }
            _ => {}
        }
    }
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return true;
    };
    let (mut found, mut any_set) = (false, false);
    tokens(&v, &mut found, &mut any_set);
    !found || any_set
}

/// Recursively find the maximum numeric [`is_expiry`] value in `v`, normalized
/// to epoch millis (values below `10^12` are treated as seconds and ×1000).
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
                if is_expiry(k)
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

/// Resolve which [`SecretStore`] engine to build from (highest precedence
/// first): the `AM_CREDENTIALS_ENGINE` env var (if non-empty), else
/// `settings.credentials.engine`, else the auto default (`"files"`).
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn credential_validity_real_claude_shape_expired_not_valid() {
        // A real Claude blob carries both clocks: `expiresAt` (access, hours)
        // and `refreshTokenExpiresAt` (refresh, thirty days). Taking the
        // refresh expiry (far in the future) instead of the access one
        // (in the past) would wrongly report `Valid` — this is the
        // regression `is_expiry` fixes.
        let past = 1_000_000_000_000i64;
        let far_future = 5_000_000_000_000i64;
        let bytes = format!(
            "{{\"claudeAiOauth\":{{\"accessToken\":\"a\",\"refreshToken\":\"r\",\
             \"expiresAt\":{past},\"refreshTokenExpiresAt\":{far_future}}}}}"
        );
        let v = credential_validity(&[blob(bytes.as_bytes())], 2_000_000_000_000);
        assert_eq!(
            v,
            Validity::Expired {
                expires_at_ms: past
            }
        );
    }

    #[test]
    fn credential_validity_blanked_shape_is_empty() {
        // What a harness leaves behind after a failed refresh / sign-out: all
        // token fields blanked, expiresAt reset to zero, but
        // refreshTokenExpiresAt still dated a month out. This must read as
        // `Empty`, not as a live (or even expired) credential.
        let future = 5_000_000_000_000i64;
        let bytes = format!(
            "{{\"claudeAiOauth\":{{\"accessToken\":\"\",\"refreshToken\":\"\",\
             \"expiresAt\":0,\"refreshTokenExpiresAt\":{future}}}}}"
        );
        let v = credential_validity(&[blob(bytes.as_bytes())], 1_000_000_000_000);
        assert_eq!(v, Validity::Empty);
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
    fn login_is_usable_false_for_blanked_blob() {
        let bytes = b"{\"claudeAiOauth\":{\"accessToken\":\"\",\"refreshToken\":\"\",\
                      \"expiresAt\":0,\"refreshTokenExpiresAt\":5000000000000}}";
        assert!(!login_is_usable(bytes));
    }

    #[test]
    fn login_is_usable_true_for_set_token() {
        let bytes = b"{\"claudeAiOauth\":{\"accessToken\":\"tok\",\"refreshToken\":\"\"}}";
        assert!(login_is_usable(bytes));
    }

    #[test]
    fn login_is_usable_true_for_non_json_bytes() {
        assert!(login_is_usable(b"not json at all"));
    }

    #[test]
    fn login_is_usable_true_for_json_naming_no_token_key() {
        assert!(login_is_usable(b"{\"apiKey\":\"sk-1\"}"));
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
    fn settings_load_parses_credentials_engine() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let path = temp.path().join("am.toml");
        std::fs::write(&path, "[credentials]\nengine = \"keychain\"\n")?;
        let settings = crate::settings::load(&path)?;
        assert_eq!(settings.credentials.engine.as_deref(), Some("keychain"));
        Ok(())
    }
}
