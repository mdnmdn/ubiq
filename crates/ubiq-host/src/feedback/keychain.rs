//! Where the feedback key override lives: one entry in the OS keychain, never the settings file.
//!
//! The URL override is a plain string in `host-settings.toml`; the key is credential material and
//! goes here — service [`SERVICE`], account [`ACCOUNT`], on `crates/ubiq`'s `host_secrets` pattern.
//! Nothing here logs a key.

/// The keychain service the override is filed under.
pub const SERVICE: &str = "dev.ubiq.feedback";
/// The one account under it.
pub const ACCOUNT: &str = "intake-key";

/// The stored override: `Ok(None)` when there is none (absent or empty), `Err` when the keychain
/// could not say — a state the caller must not read as "no override", and should retry.
pub fn load() -> Result<Option<String>, String> {
    let unread =
        |error: keyring::Error| format!("the feedback key was not read from the keychain: {error}");
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(unread)?;
    match entry.get_password() {
        Ok(key) => Ok(Some(key).filter(|key| !key.is_empty())),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => {
            tracing::debug!("the feedback key override was not read: {error}");
            Err(unread(error))
        }
    }
}

/// File the override, replacing whatever was there.
pub fn save(key: &str) -> Result<(), String> {
    keyring::Entry::new(SERVICE, ACCOUNT)
        .and_then(|entry| entry.set_password(key))
        .map_err(|error| format!("the key was not kept in the keychain: {error}"))
}

/// Drop the override. One that was never there is already dropped.
pub fn delete() -> Result<(), String> {
    match keyring::Entry::new(SERVICE, ACCOUNT).and_then(|entry| entry.delete_credential()) {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!(
            "the key was not removed from the keychain: {error}"
        )),
    }
}
