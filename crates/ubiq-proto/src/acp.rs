//! What an ACP agent said it can do, as it crosses the bus.
//!
//! The mirror of `agent_manager::io::acp_caps` — the library owns the protocol facts (which key
//! means what, which encoding says "supported") and the host maps its answer onto these types,
//! exactly as [`crate::quota`] mirrors the library's quota vocabulary. The UI crate does not
//! depend on `agent-manager` and never will, so the wire carries its own copy.
//!
//! Three properties decide the shape:
//!
//! - **It is a harness fact, not a conversation one.** An ACP agent states this once, in its
//!   `initialize` answer, and every session behind the same harness id hears the same thing. So it
//!   is keyed on the harness and survives the agent that discovered it, which is what lets the
//!   harness settings show it with nothing running.
//! - **It is a list of rows, not a struct of named booleans.** Adding a capability is never a
//!   breaking change in ACP, so a capability this build has never heard of still has a row. A
//!   struct with one field per key would silently drop it.
//! - **It is discovered, never assumed.** There is no probe that asks an agent this question on
//!   its own; the answer exists because a conversation's handshake happened. A harness that has
//!   never been conversed with has no record, and saying so is the honest answer — see
//!   [`crate::messages::Message::AcpCapabilities`].

use serde::{Deserialize, Serialize};

/// One ACP agent's `initialize` answer, read for display.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpCapabilitiesRecord {
    /// The protocol version the agent and the client settled on. `1` today, always.
    pub protocol_version: i64,
    /// Who answered — the agent's own `agentInfo`, where it sent one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AcpImplementationRecord>,
    /// The capability tree, flattened into groups of named rows in the order to list them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<AcpCapabilityGroupRecord>,
    /// The authentication methods the agent advertised, in its own order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auth_methods: Vec<AcpAuthMethodRecord>,
    /// When the handshake that produced this happened, in epoch milliseconds. It is the record's
    /// only staleness signal, and deliberately a weak one: an answer from an older build of the
    /// agent is still the last true answer, so nothing invalidates on it — it is shown so a reader
    /// can see how old the answer is. Which build answered is [`AcpImplementationRecord::version`],
    /// which the agent states itself.
    #[serde(default)]
    pub discovered_ms: i64,
}

/// Who is on the other end of the pipe.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpImplementationRecord {
    /// The agent's programmatic id.
    pub name: String,
    /// Its human-readable name, where it sent one — what to show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The build it reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl AcpImplementationRecord {
    /// What to call this agent on screen: its `title` where it sent one, its `name` otherwise.
    pub fn display_name(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.name)
    }
}

/// One heading in the list, with its rows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpCapabilityGroupRecord {
    /// What the heading says, e.g. `Prompt content`.
    pub label: String,
    /// The rows under it, in order.
    pub entries: Vec<AcpCapabilityRecord>,
}

/// One capability, as a row.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpCapabilityRecord {
    /// The wire key path, e.g. `promptCapabilities.image` — the only part that is not prose, and
    /// the id an element is keyed on.
    pub id: String,
    /// What the row says.
    pub label: String,
    /// Whether the agent advertised it.
    pub supported: bool,
    /// What having it means, in one line. Empty for a capability the library does not recognise.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// One authentication method the agent advertised. **No credential material** — an id, a name and
/// a line of prose, which is all `authMethods` carries.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpAuthMethodRecord {
    /// The agent's own id for the method.
    pub id: String,
    /// What the row says.
    pub name: String,
    /// The agent's own line about it, where it sent one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether the agent nominated this one as the method that needs no interaction.
    #[serde(default)]
    pub default: bool,
}
