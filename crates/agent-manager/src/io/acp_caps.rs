//! Reading an ACP agent's `initialize` result as a list a reader can be shown.
//!
//! The handshake in [`super::acp_client`] already asks `initialize` and keeps the
//! answer's `agentCapabilities` to gate what it sends. This module turns that same
//! answer — once, at the same moment — into [`AcpCapabilities`]: the agent's own
//! identity, the protocol version it settled on, and the capability tree flattened
//! into named rows. **It is the only reading of that tree meant to be displayed**,
//! so a surface never parses the raw JSON and never carries its own list of what a
//! capability is called.
//!
//! Two rules from `_docs/references/acp-protocol.md` decide how a row is read:
//!
//! - **Omitted, `null` and `false` all mean unsupported.** The newer nested
//!   capabilities encode "supported" as a present object (`{}`), the older ones as
//!   a plain `true`; [`supported_flag`] and [`supported_object`] are those two
//!   readings, and every row picks one.
//! - **Adding a capability is never a breaking change**, so an agent may advertise
//!   something this build has never heard of. Those land in an `Other` group under
//!   their own wire key rather than being dropped — a panel that silently omits what
//!   it does not recognise is a panel that lies as the protocol grows.
//!
//! This module is **core** (no feature gate): `serde_json` and `serde` only.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What an ACP agent said about itself during `initialize`.
///
/// Built once per handshake and thereafter a record, not a probe: nothing here is
/// re-read from the agent, and nothing here is per-session. Two agents behind the
/// same harness id answer the same thing, which is what makes it cacheable against
/// the harness rather than against a run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpCapabilities {
    /// The protocol version the agent answered. This client speaks `1` and refuses
    /// anything else, so today it is always `1` — recorded anyway, because the
    /// refusal is what would change if a version 2 ever lands.
    pub protocol_version: i64,
    /// The agent's `agentInfo`, where it sent one. `title` is the name to show and
    /// `name` the fallback, per the spec's own note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AcpImplementation>,
    /// The capability tree, flattened into groups of named rows in the order a
    /// panel lists them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<AcpCapabilityGroup>,
    /// The `authMethods` the agent advertised, in the order it listed them. Empty
    /// is the normal case for an agent that settles its own credentials.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auth_methods: Vec<AcpAuthMethod>,
}

/// An ACP `Implementation` — who is on the other end of the pipe.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpImplementation {
    /// The programmatic id, always sent.
    pub name: String,
    /// The human-readable name, preferred for display where it is sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The build the agent reported, e.g. `1.18.28`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl AcpImplementation {
    /// What to call this agent on screen: its `title` where it sent one, its `name`
    /// otherwise — the fallback the spec names.
    pub fn display_name(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.name)
    }
}

/// One heading in the list, with the rows under it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpCapabilityGroup {
    /// What the heading says, e.g. `Prompt content`.
    pub label: String,
    /// The rows under it, in the order they are listed.
    pub entries: Vec<AcpCapability>,
}

/// One capability, as a row: what it is called, whether the agent has it, and what
/// it buys.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpCapability {
    /// The wire key path, e.g. `promptCapabilities.image` — stable, and the only
    /// part of a row that is not prose.
    pub id: String,
    /// What the row says.
    pub label: String,
    /// Whether the agent advertised it.
    pub supported: bool,
    /// What having it means, in one line. Empty for a capability this build does
    /// not recognise, because inventing a description for one would be a guess.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// One `AuthMethod` the agent advertised.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpAuthMethod {
    /// The agent's own id for the method — what an `authenticate` names.
    pub id: String,
    /// What the row says; the `id` where the agent sent no name.
    pub name: String,
    /// The agent's own line about it, where it sent one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether the agent nominated this one as the method that needs no
    /// interaction (`_meta.defaultAuthMethodId`) — the one, and the only one, the
    /// bridge will use on its own.
    #[serde(default)]
    pub default: bool,
}

/// The older encoding: a plain boolean, where absent means unsupported.
fn supported_flag(root: &Value, path: &[&str]) -> bool {
    at(root, path).and_then(Value::as_bool).unwrap_or(false)
}

/// The newer encoding: a *present object* means supported, `null` and absent both
/// mean not. `{}` is the whole signal — see the module docs.
fn supported_object(root: &Value, path: &[&str]) -> bool {
    matches!(at(root, path), Some(value) if value.is_object())
}

fn at<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(root, |value, key| value.get(key))
}

/// Every capability key this build knows, so [`other_group`] can tell a new one
/// from a listed one. Wire keys, in `agentCapabilities`' own spelling.
const KNOWN_KEYS: &[&str] = &[
    "loadSession",
    "promptCapabilities",
    "mcpCapabilities",
    "sessionCapabilities",
    "auth",
    "_meta",
];

/// Read one `initialize` result into the record two surfaces draw.
///
/// `init` is the response's `result` object, verbatim. A missing or malformed field
/// is read as "unsupported" rather than as an error: this describes an agent, it
/// does not validate one, and the handshake that produced `init` has already
/// refused anything it could not speak.
pub fn read_initialize(init: &Value) -> AcpCapabilities {
    let caps = init
        .get("agentCapabilities")
        .cloned()
        .unwrap_or(Value::Null);
    AcpCapabilities {
        protocol_version: init
            .get("protocolVersion")
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        agent: agent_info(init),
        groups: groups(&caps),
        auth_methods: auth_methods(init),
    }
}

fn agent_info(init: &Value) -> Option<AcpImplementation> {
    let info = init.get("agentInfo")?.as_object()?;
    Some(AcpImplementation {
        name: info
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        title: info.get("title").and_then(Value::as_str).map(String::from),
        version: info
            .get("version")
            .and_then(Value::as_str)
            .map(String::from),
    })
}

/// The `AgentCapabilities` tree as four groups of rows, plus whatever this build
/// has never heard of.
fn groups(caps: &Value) -> Vec<AcpCapabilityGroup> {
    let row = |id: &str, label: &str, description: &str, supported: bool| AcpCapability {
        id: id.to_string(),
        label: label.to_string(),
        supported,
        description: description.to_string(),
    };
    let mut groups = vec![
        AcpCapabilityGroup {
            label: "Sessions".to_string(),
            entries: vec![
                row(
                    "loadSession",
                    "Load session",
                    "`session/load` — a past conversation is replayed rather than started again.",
                    supported_flag(caps, &["loadSession"]),
                ),
                row(
                    "sessionCapabilities.list",
                    "List sessions",
                    "`session/list` — the agent enumerates the sessions it holds.",
                    supported_object(caps, &["sessionCapabilities", "list"]),
                ),
                row(
                    "sessionCapabilities.resume",
                    "Resume session",
                    "`session/resume` — a session continues where it stopped.",
                    supported_object(caps, &["sessionCapabilities", "resume"]),
                ),
                row(
                    "sessionCapabilities.delete",
                    "Delete session",
                    "`session/delete` — the agent forgets a session on request.",
                    supported_object(caps, &["sessionCapabilities", "delete"]),
                ),
                row(
                    "sessionCapabilities.close",
                    "Close session",
                    "`session/close` — a session is released without ending the process.",
                    supported_object(caps, &["sessionCapabilities", "close"]),
                ),
                row(
                    "sessionCapabilities.additionalDirectories",
                    "Additional directories",
                    "A session may be given roots beyond its `cwd`.",
                    supported_object(caps, &["sessionCapabilities", "additionalDirectories"]),
                ),
            ],
        },
        AcpCapabilityGroup {
            label: "Prompt content".to_string(),
            entries: vec![
                row(
                    "promptCapabilities.image",
                    "Images",
                    "An `image` content block may be sent in a prompt.",
                    supported_flag(caps, &["promptCapabilities", "image"]),
                ),
                row(
                    "promptCapabilities.audio",
                    "Audio",
                    "An `audio` content block may be sent in a prompt.",
                    supported_flag(caps, &["promptCapabilities", "audio"]),
                ),
                row(
                    "promptCapabilities.embeddedContext",
                    "Embedded context",
                    "A `resource` content block may carry a file's contents inline.",
                    supported_flag(caps, &["promptCapabilities", "embeddedContext"]),
                ),
            ],
        },
        AcpCapabilityGroup {
            label: "MCP transports".to_string(),
            entries: vec![
                row(
                    "mcpCapabilities.http",
                    "HTTP",
                    "An MCP server may be given as `type: \"http\"`.",
                    supported_flag(caps, &["mcpCapabilities", "http"]),
                ),
                row(
                    "mcpCapabilities.sse",
                    "SSE",
                    "An MCP server may be given as `type: \"sse\"` — deprecated by MCP itself.",
                    supported_flag(caps, &["mcpCapabilities", "sse"]),
                ),
            ],
        },
        AcpCapabilityGroup {
            label: "Authentication".to_string(),
            entries: vec![row(
                "auth.logout",
                "Logout",
                "The agent exposes a `logout` method.",
                supported_object(caps, &["auth", "logout"]),
            )],
        },
    ];
    groups.extend(other_group(caps));
    groups
}

/// Anything in `agentCapabilities` that [`KNOWN_KEYS`] does not name, one row per
/// key, read with whichever encoding the value itself uses. A row here is the
/// protocol having grown past this build, which is expected rather than an error.
fn other_group(caps: &Value) -> Option<AcpCapabilityGroup> {
    let entries: Vec<AcpCapability> = caps
        .as_object()?
        .iter()
        .filter(|(key, _)| !KNOWN_KEYS.contains(&key.as_str()))
        .map(|(key, value)| AcpCapability {
            id: key.clone(),
            label: key.clone(),
            // Both encodings at once: a bare `true`, or a present object.
            supported: value.as_bool().unwrap_or_else(|| value.is_object()),
            description: String::new(),
        })
        .collect();
    (!entries.is_empty()).then_some(AcpCapabilityGroup {
        label: "Other".to_string(),
        entries,
    })
}

fn auth_methods(init: &Value) -> Vec<AcpAuthMethod> {
    let default_id = init
        .get("_meta")
        .and_then(|meta| meta.get("defaultAuthMethodId"))
        .and_then(Value::as_str);
    init.get("authMethods")
        .and_then(Value::as_array)
        .map(|methods| {
            methods
                .iter()
                .filter_map(|method| {
                    let id = method.get("id").and_then(Value::as_str)?;
                    Some(AcpAuthMethod {
                        id: id.to_string(),
                        name: method
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or(id)
                            .to_string(),
                        description: method
                            .get("description")
                            .and_then(Value::as_str)
                            .map(String::from),
                        default: default_id == Some(id),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row<'a>(caps: &'a AcpCapabilities, id: &str) -> &'a AcpCapability {
        caps.groups
            .iter()
            .flat_map(|group| &group.entries)
            .find(|entry| entry.id == id)
            .unwrap_or_else(|| panic!("no row '{id}' in {:?}", caps.groups))
    }

    #[test]
    fn an_empty_result_answers_every_row_unsupported() {
        let caps = read_initialize(&json!({"protocolVersion": 1}));
        assert_eq!(caps.protocol_version, 1);
        assert!(caps.agent.is_none());
        assert!(caps.auth_methods.is_empty());
        assert!(
            caps.groups
                .iter()
                .flat_map(|group| &group.entries)
                .all(|entry| !entry.supported),
            "{:?}",
            caps.groups
        );
    }

    /// The tri-state that is really a bi-state: `{}` is supported, `null` and
    /// absent are not, and a `false` boolean is not either.
    #[test]
    fn a_present_object_is_support_and_null_is_not() {
        let caps = read_initialize(&json!({
            "protocolVersion": 1,
            "agentCapabilities": {
                "loadSession": true,
                "promptCapabilities": {"image": true, "audio": false},
                "sessionCapabilities": {"list": {}, "delete": null},
            },
        }));
        assert!(row(&caps, "loadSession").supported);
        assert!(row(&caps, "promptCapabilities.image").supported);
        assert!(!row(&caps, "promptCapabilities.audio").supported);
        assert!(!row(&caps, "promptCapabilities.embeddedContext").supported);
        assert!(row(&caps, "sessionCapabilities.list").supported);
        assert!(!row(&caps, "sessionCapabilities.delete").supported);
        assert!(!row(&caps, "sessionCapabilities.close").supported);
    }

    /// Adding a capability is never a breaking change, so one this build has never
    /// heard of is listed rather than dropped.
    #[test]
    fn an_unknown_capability_lands_in_other() {
        let caps = read_initialize(&json!({
            "protocolVersion": 1,
            "agentCapabilities": {"loadSession": true, "timeTravel": {}},
        }));
        let other = caps
            .groups
            .iter()
            .find(|group| group.label == "Other")
            .expect("an Other group");
        assert_eq!(other.entries.len(), 1);
        assert_eq!(other.entries[0].id, "timeTravel");
        assert!(other.entries[0].supported);
        assert!(other.entries[0].description.is_empty());
    }

    #[test]
    fn the_nominated_auth_method_is_the_one_marked_default() {
        let caps = read_initialize(&json!({
            "protocolVersion": 1,
            "authMethods": [
                {"id": "cached_token", "name": "Cached token"},
                {"id": "oauth", "name": "OAuth", "description": "Opens a browser"},
            ],
            "_meta": {"defaultAuthMethodId": "cached_token"},
        }));
        assert_eq!(caps.auth_methods.len(), 2);
        assert!(caps.auth_methods[0].default);
        assert!(!caps.auth_methods[1].default);
        assert_eq!(
            caps.auth_methods[1].description.as_deref(),
            Some("Opens a browser")
        );
    }

    #[test]
    fn agent_info_prefers_the_title_it_sent() {
        let caps = read_initialize(&json!({
            "protocolVersion": 1,
            "agentInfo": {"name": "opencode", "title": "opencode", "version": "1.18.28"},
        }));
        let agent = caps.agent.unwrap();
        assert_eq!(agent.display_name(), "opencode");
        assert_eq!(agent.version.as_deref(), Some("1.18.28"));

        let caps = read_initialize(&json!({
            "protocolVersion": 1,
            "agentInfo": {"name": "grok", "version": "1.0.13"},
        }));
        assert_eq!(caps.agent.unwrap().display_name(), "grok");
    }
}
