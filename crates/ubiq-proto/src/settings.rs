//! Application settings as they cross the bus: which half owns the schema, and the host's own record.
//!
//! View state is a different store — [`crate::projects::Scope`] and the preference messages. Settings
//! are how the application behaves. Two layers, because the host must never parse what it does
//! not own, and must parse what it does.

use serde::{Deserialize, Serialize};

/// Which half owns the schema of a settings blob.
///
/// The Ui layer is opaque to the host: a string it writes down and hands back. The Host layer is
/// the host's to parse and act on. Harness definitions are neither — they belong to agent-manager.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingsLayer {
    /// Schema owned by the interface. The host stores the blob and never looks inside.
    Ui,
    /// Schema owned by the host. The host parses it; a blob it cannot read is an error, not a
    /// discarded default.
    Host,
}

/// The host-owned settings record. Empty of fields this build acts on; `schema` is the hook a
/// later field hangs from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostSettings {
    pub schema: u32,
}

/// The shape this host writes and understands.
pub const HOST_SETTINGS_SCHEMA: u32 = 1;

impl Default for HostSettings {
    fn default() -> Self {
        Self {
            schema: HOST_SETTINGS_SCHEMA,
        }
    }
}
