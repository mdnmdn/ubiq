//! Application settings as the host runs them: two layers, two recovery rules.
//!
//! The Ui layer is opaque — a string the host writes down and hands back. The Host layer is this
//! half's to parse: a blob it cannot read is preserved and reported, not discarded.

use std::sync::Mutex;

use ubiq_proto::messages::Message;
use ubiq_proto::settings::{HOST_SETTINGS_SCHEMA, HostSettings, SettingsLayer};

use crate::reply::Reply;
use crate::store::{SettingsStore, StoreError};

/// The two settings files, behind one trait.
pub struct Settings {
    store: Box<dyn SettingsStore>,
    /// Held across every host-layer read-modify-write.
    ///
    /// Two writers reach this record: a `SetSettings` on the coordinator's thread, and a connector
    /// flow on its own. Without the lock, a flow storing a connection could interleave with a
    /// settings toggle and one of them would be lost.
    ///
    // ponytail: one lock over the whole host layer, because a write is a whole-file rewrite
    // anyway. Per-field locks only if a flow is ever seen waiting on a toggle.
    writing: Mutex<()>,
}

impl Settings {
    pub fn open(store: Box<dyn SettingsStore>) -> Self {
        Self {
            store,
            writing: Mutex::new(()),
        }
    }

    /// The host layer, parsed, for the parts of the host that act on it.
    ///
    /// An absent record answers the defaults, and so does one this build cannot
    /// read — the error belongs to whoever asked for it over the bus, and a
    /// spawn is not the place to relitigate it. What is on disk is preserved
    /// either way; nothing here writes.
    pub fn host(&self) -> HostSettings {
        match self.store.get(SettingsLayer::Host) {
            Ok(Some(value)) => parse_host(&value).unwrap_or_default(),
            Ok(None) => HostSettings::default(),
            Err(error) => {
                tracing::warn!("host settings were not read, using defaults: {error}");
                HostSettings::default()
            }
        }
    }

    pub fn get(&self, layer: SettingsLayer) -> Reply {
        match self.store.get(layer) {
            Ok(value) => Reply::Asker(Message::Settings { layer, value }),
            Err(error) => Reply::Asker(Message::SettingsError {
                layer,
                error: error.to_string(),
            }),
        }
    }

    /// Store a blob. The Ui layer answers nothing: a failed write is a log line. The Host layer
    /// is parsed first, and a blob this build will not take answers [`Message::SettingsError`].
    pub fn set(&self, layer: SettingsLayer, value: String) -> Vec<Reply> {
        match layer {
            SettingsLayer::Ui => {
                if let Err(error) = self.store.set(layer, &value) {
                    tracing::warn!("ui settings were not durable: {error}");
                }
                Vec::new()
            }
            SettingsLayer::Host => {
                let _guard = self.lock();
                match parse_host(&value) {
                    Err(error) => vec![Reply::Asker(Message::SettingsError { layer, error })],
                    Ok(mut settings) => {
                        // The three fields the host owns. The interface's copy of them is as old as
                        // the dialog it was opened with, and a flow that completed in between wrote
                        // the real one — so what came over the bus for these is discarded and what
                        // is on disk is kept. Everything else in the blob is the interface's to
                        // write, and is written unchanged.
                        let held = self.host();
                        settings.connections = held.connections;
                        settings.oauth_apps = held.oauth_apps;
                        settings.trusted_certs = held.trusted_certs;
                        match self.write(&settings) {
                            Ok(()) => Vec::new(),
                            Err(error) => {
                                vec![Reply::Asker(Message::SettingsError { layer, error })]
                            }
                        }
                    }
                }
            }
        }
    }

    /// Read, change and write the host layer.
    ///
    /// The only way the connector fields move, and the only writer a flow thread has. Answers the
    /// record as it now stands so a caller can broadcast it without a second read.
    ///
    // ponytail: `host()` answers defaults for a record it cannot parse, so a corrupt
    // host-settings.toml means the next connector write starts from empty and the connections it
    // held are gone. That is already this file's behaviour for the other fields; it matters more
    // here, and it is the argument for connectors keeping their own store if it ever bites.
    pub fn update_host(
        &self,
        change: impl FnOnce(&mut HostSettings),
    ) -> Result<HostSettings, String> {
        let _guard = self.lock();
        let mut settings = self.host();
        settings.schema = HOST_SETTINGS_SCHEMA;
        change(&mut settings);
        self.write(&settings)?;
        Ok(settings)
    }

    /// Serialise and store one host record. The one place a host record is written, so `set` and
    /// [`Self::update_host`] cannot drift apart.
    fn write(&self, settings: &HostSettings) -> Result<(), String> {
        let value = serde_json::to_string(settings).map_err(|error| error.to_string())?;
        self.store
            .set(SettingsLayer::Host, &value)
            .map_err(|error| error.to_string())
    }

    /// A poisoned lock is not a reason to refuse a write: nothing here holds an invariant across
    /// the guard, so the panic that poisoned it left the record intact.
    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.writing
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

fn parse_host(value: &str) -> Result<HostSettings, String> {
    let settings: HostSettings = serde_json::from_str(value).map_err(|error| error.to_string())?;
    if settings.schema > HOST_SETTINGS_SCHEMA {
        return Err(StoreError::UnknownVersion {
            path: "host-settings.toml".into(),
            found: settings.schema,
            supported: HOST_SETTINGS_SCHEMA,
        }
        .to_string());
    }
    Ok(settings)
}
