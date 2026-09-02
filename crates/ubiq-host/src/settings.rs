//! Application settings as the host runs them: two layers, two recovery rules.
//!
//! The Ui layer is opaque — a string the host writes down and hands back. The Host layer is this
//! half's to parse: a blob it cannot read is preserved and reported, not discarded.

use ubiq_proto::messages::Message;
use ubiq_proto::settings::{HOST_SETTINGS_SCHEMA, HostSettings, SettingsLayer};

use crate::reply::Reply;
use crate::store::{SettingsStore, StoreError};

/// The two settings files, behind one trait.
pub struct Settings {
    store: Box<dyn SettingsStore>,
}

impl Settings {
    pub fn open(store: Box<dyn SettingsStore>) -> Self {
        Self { store }
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
            SettingsLayer::Host => match parse_host(&value) {
                Err(error) => vec![Reply::Asker(Message::SettingsError { layer, error })],
                Ok(_) => match self.store.set(layer, &value) {
                    Ok(()) => Vec::new(),
                    Err(error) => vec![Reply::Asker(Message::SettingsError {
                        layer,
                        error: error.to_string(),
                    })],
                },
            },
        }
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
