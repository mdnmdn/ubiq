//! Where a remote host's bearer token lives: the OS keychain, never the settings file.
//!
//! [`SavedRemoteHost`](ubiq_proto::settings::SavedRemoteHost) carries the address, the scheme
//! and the trust flag — everything needed to *offer* a reconnect — and the token this module
//! keeps is what *performs* one. One entry per saved host, keyed by its stable `id` (or by
//! `legacy:<address>` for records written before ids existed). A token is a
//! [`Secret`](ubiq_proto::messages::Secret)-shaped value at every boundary here: loaded only
//! for the duration of a dial, never logged, never taped.
//!
//! No keychain on the machine, or one that refuses, is "no saved token" rather than an error —
//! the caller falls back to asking the user, the same modal it would have raised anyway.

/// The keychain service every remote-host entry is filed under.
const SERVICE: &str = "dev.ubiq.remote-host";

/// The key a saved entry's token is filed under: its stable id, or the legacy address key for
/// records written before ids existed.
pub fn key_for(save_id: &str, address: &str) -> String {
    if save_id.is_empty() {
        format!("legacy:{address}")
    } else {
        save_id.to_string()
    }
}

/// File a token after a dial proved it good. Overwrites whatever was there: a token is only
/// ever saved on success, so what is here is always the last one the host accepted.
pub fn save_token(key: &str, token: &str) {
    if let Ok(entry) = keyring::Entry::new(SERVICE, key)
        && let Err(error) = entry.set_password(token)
    {
        tracing::debug!("remote host token was not kept: {error}");
    }
}

/// The token a reconnect should dial with, or `None` when there is none to dial with — absent,
/// or the store refused to say. Both read the same to the caller: ask the user.
pub fn load_token(key: &str) -> Option<String> {
    keyring::Entry::new(SERVICE, key)
        .ok()?
        .get_password()
        .ok()
        .filter(|token| !token.is_empty())
}

/// Forget a token, as forgetting its saved host does. A store that refuses is a log line: the
/// record is gone either way, and a leftover entry for an id nothing names is inert.
pub fn delete_token(key: &str) {
    if let Ok(entry) = keyring::Entry::new(SERVICE, key)
        && let Err(error) = entry.delete_credential()
    {
        tracing::debug!("remote host token was not forgotten: {error}");
    }
}
