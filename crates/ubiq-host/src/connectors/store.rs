//! Where a connection's token lives, and what it says about itself.
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
use ubiq_proto::ids::ConnectionId;
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

    pub fn set_app_secret(
        &self,
        provider: ProviderId,
        origin: Option<&str>,
        secret: &str,
    ) -> Result<(), String> {
        self.inner
            .set(
                &app_key(provider, origin),
                &[blob(secret.as_bytes().to_vec())],
            )
            .map_err(|error| error.to_string())
    }

    pub fn clear_app_secret(
        &self,
        provider: ProviderId,
        origin: Option<&str>,
    ) -> Result<(), String> {
        self.inner
            .delete(&app_key(provider, origin))
            .map_err(|error| error.to_string())
    }

    pub fn app_secret(&self, provider: ProviderId, origin: Option<&str>) -> Option<String> {
        let blobs = self.inner.get(&app_key(provider, origin)).ok().flatten()?;
        let blob = blobs.first()?;
        String::from_utf8(blob.bytes.clone()).ok()
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

/// Where an application's client *secret* is filed — a different namespace, because it identifies
/// Ubiq rather than the user, and there is one per instance rather than one per connection.
pub fn app_key(provider: ProviderId, origin: Option<&str>) -> CredentialId {
    CredentialId {
        harness: format!("connector-app:{}", slug(provider)),
        name: origin.unwrap_or("cloud").to_string(),
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
