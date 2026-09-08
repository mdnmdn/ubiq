//! Where a connection's token lives, and what it says about itself.
//!
//! Three kinds of secret are filed here, in three namespaces that cannot collide: a connection's
//! token ([`key`]), the client secret of an OAuth registration Ubiq authenticates *as*
//! ([`app_key`]), and an API provider's key ([`ai_key`]). One store rather than three, because
//! whether the platform's keychain works at all is one fact and [`Store::usable`] answers it once —
//! and because a second [`OsSecretStore`] over the same directory would be a second answer to it.
//!
//! **The engine is chosen here, not inherited.** `agent_manager::credentials::build_secret_store`
//! resolves an engine from `AM_CREDENTIALS_ENGINE` and falls back to plaintext files — fine for a
//! harness login the user captured from their own home directory, and wrong for a bearer token this
//! application obtained on their behalf. So [`OsSecretStore`] is constructed directly: the
//! platform's real, encrypted secret service or nothing.
//!
//! A token is stored as one `token.json` blob rather than as a bare string, and that is what makes
//! [`credential_validity`] work here unmodified: it looks for a numeric `*expire*` field anywhere in
//! a blob's JSON, so `expires_at` is found without either half knowing about the other.

use std::path::Path;
use std::sync::OnceLock;

use agent_manager::Validity;
use agent_manager::credentials::{
    CredentialBlob, CredentialId, OsSecretStore, SecretStore, credential_validity,
};
use serde::{Deserialize, Serialize};
use ubiq_proto::connectors::ProviderId;
use ubiq_proto::ids::{AiProviderId, ConnectionId, OauthAppId};
use ubiq_proto::messages::LoginStatus;

/// The one file a connection's credential is made of.
const BLOB: &str = "token.json";

/// What a flow obtained, as it is written down.
///
/// Every field but the access token is optional because providers disagree about all of them: a
/// personal access token has no refresh token and no expiry, and a device flow's answer may name a
/// scope set that is not the one asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Epoch seconds. Named so `credential_validity` finds it — it matches any numeric key
    /// containing `expire`, and treats a value below 10^12 as seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// The connector family's secret store.
pub struct Store {
    inner: OsSecretStore,
    /// Whether the platform's store actually works, answered once.
    ///
    // ponytail: probed on first use rather than at construction, so building a host does no
    // keychain I/O and a test that never connects never touches the user's login keychain. Every
    // flow asks before it does anything, which is still before a browser opens.
    usable: OnceLock<Result<(), String>>,
}

impl Store {
    /// The store under Ubiq's config root. Constructing does no I/O.
    pub fn open(root: &Path) -> Self {
        Self {
            inner: OsSecretStore::new(root.join("keychain")),
            usable: OnceLock::new(),
        }
    }

    /// Whether a token can actually be kept. A round trip, because a store that lists and cannot
    /// write is the failure worth catching — and catching it here means no browser ever opens for
    /// a flow whose token would have nowhere to go.
    pub fn usable(&self) -> Result<(), String> {
        self.usable
            .get_or_init(|| {
                let probe = CredentialId {
                    harness: "connector:probe".to_string(),
                    name: "probe".to_string(),
                };
                let blobs = [blob(b"{}".to_vec())];
                self.inner
                    .set(&probe, &blobs)
                    .and_then(|()| self.inner.delete(&probe))
                    .map_err(|error| error.to_string())
            })
            .clone()
    }

    pub fn put(&self, provider: ProviderId, id: ConnectionId, token: &Token) -> Result<(), String> {
        let bytes = serde_json::to_vec(token).map_err(|error| error.to_string())?;
        self.inner
            .set(&key(provider, id), &[blob(bytes)])
            .map_err(|error| error.to_string())
    }

    pub fn delete(&self, provider: ProviderId, id: ConnectionId) -> Result<(), String> {
        self.inner
            .delete(&key(provider, id))
            .map_err(|error| error.to_string())
    }

    /// What the stored credential claims about itself. No network: `Valid` means "not expired",
    /// not "will work".
    pub fn status(&self, provider: ProviderId, id: ConnectionId, now_ms: i64) -> LoginStatus {
        let blobs = self
            .inner
            .get(&key(provider, id))
            .ok()
            .flatten()
            .unwrap_or_default();
        validity(credential_validity(&blobs, now_ms))
    }

    /// The same reading, over blobs that are already in hand.
    ///
    /// Split out so the mapping can be exercised without a keychain: what a credential says about
    /// itself is a pure function of its bytes and the clock.
    pub fn status_of(blobs: &[Vec<u8>], now_ms: i64) -> LoginStatus {
        let blobs: Vec<CredentialBlob> = blobs.iter().cloned().map(blob).collect();
        validity(credential_validity(&blobs, now_ms))
    }

    /// The access token itself, for a probe.
    pub fn token(&self, provider: ProviderId, id: ConnectionId) -> Option<Token> {
        let blobs = self.inner.get(&key(provider, id)).ok().flatten()?;
        let blob = blobs.iter().find(|blob| blob.name == BLOB)?;
        serde_json::from_slice(&blob.bytes).ok()
    }

    /// File an API provider's key. One blob like every other secret here, so
    /// [`credential_validity`] reads it as `Unknown` — a bare key claims no expiry, and nothing
    /// asks it to.
    pub fn set_ai_key(&self, provider: AiProviderId, key: &str) -> Result<(), String> {
        self.inner
            .set(&ai_key(provider), &[blob(key.as_bytes().to_vec())])
            .map_err(|error| error.to_string())
    }

    pub fn clear_ai_key(&self, provider: AiProviderId) -> Result<(), String> {
        self.inner
            .delete(&ai_key(provider))
            .map_err(|error| error.to_string())
    }

    /// The key itself, for a request about to be made. `None` means there is none filed, which is
    /// a provider that cannot be called — never an empty key that is tried and refused.
    pub fn ai_key_value(&self, provider: AiProviderId) -> Option<String> {
        let blobs = self.inner.get(&ai_key(provider)).ok().flatten()?;
        let blob = blobs.first()?;
        String::from_utf8(blob.bytes.clone()).ok()
    }

    /// Whether a key is filed, without reading one. This is what a settings row is drawn from, so
    /// the material never leaves the store for a question about its presence.
    pub fn has_ai_key(&self, provider: AiProviderId) -> bool {
        self.inner
            .get(&ai_key(provider))
            .ok()
            .flatten()
            .is_some_and(|blobs| !blobs.is_empty())
    }

    pub fn set_app_secret(&self, app: OauthAppId, secret: &str) -> Result<(), String> {
        self.inner
            .set(&app_key(app), &[blob(secret.as_bytes().to_vec())])
            .map_err(|error| error.to_string())
    }

    pub fn clear_app_secret(&self, app: OauthAppId) -> Result<(), String> {
        self.inner
            .delete(&app_key(app))
            .map_err(|error| error.to_string())
    }

    pub fn app_secret(&self, app: OauthAppId) -> Option<String> {
        let blobs = self.inner.get(&app_key(app)).ok().flatten()?;
        let blob = blobs.first()?;
        String::from_utf8(blob.bytes.clone()).ok()
    }
}

/// Where an API provider's key is filed — a third namespace, because a provider key is neither a
/// user's identity at a service nor Ubiq's own registration: it is material Ubiq spends when the
/// user asks it to write a line of prose.
///
/// Keyed by the record's id alone. The kind is not in the key, so changing a provider's kind — an
/// OpenAI-compatible endpoint the user re-points at Anthropic — keeps the key it already had
/// rather than orphaning it under the old spelling.
pub fn ai_key(provider: AiProviderId) -> CredentialId {
    CredentialId {
        harness: "ai-provider".to_string(),
        name: provider.to_string(),
    }
}

/// Where one connection's token is filed.
///
/// The harness slot names the provider and the name is the connection's own id, so several
/// identities at one provider are several credentials with nothing to collide over.
pub fn key(provider: ProviderId, id: ConnectionId) -> CredentialId {
    CredentialId {
        harness: format!("connector:{}", slug(provider)),
        name: id.to_string(),
    }
}

/// Where a registration's client *secret* is filed — a different namespace, because it identifies
/// Ubiq rather than the user, and there is one per registration rather than one per connection.
///
/// Keyed by the registration's id and not by its provider and instance, which is what lets two
/// registrations on one install hold two different secrets.
pub fn app_key(app: OauthAppId) -> CredentialId {
    CredentialId {
        harness: "connector-app".to_string(),
        name: app.to_string(),
    }
}

/// The same `Validity` → `LoginStatus` reading the harness half already uses, so a connection and a
/// harness login cannot come to disagree about what "expired" means.
pub fn validity(validity: Validity) -> LoginStatus {
    match validity {
        Validity::Empty => LoginStatus::Missing,
        Validity::Valid {
            expires_at_ms: Some(expires_at_ms),
        } => LoginStatus::Valid { expires_at_ms },
        Validity::Valid {
            expires_at_ms: None,
        }
        | Validity::Unknown => LoginStatus::Unknown,
        Validity::Expired { expires_at_ms } => LoginStatus::Expired { expires_at_ms },
    }
}

fn blob(bytes: Vec<u8>) -> CredentialBlob {
    CredentialBlob {
        name: BLOB.to_string(),
        rel_path: BLOB.into(),
        bytes,
    }
}

/// The provider's name in a credential id. Serde already knows it, and asking serde keeps the two
/// spellings from drifting.
fn slug(provider: ProviderId) -> String {
    serde_json::to_value(provider)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{provider:?}").to_lowercase())
}
