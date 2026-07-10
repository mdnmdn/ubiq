use crate::messages::AgentTypeInfo;
use agent_manager::harness::{self, Harness};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// A spawnable harness/agent type known to Ubiq.
///
/// Definitions are seeded from the portable `agent_manager` harness registry
/// (the single source of truth for which harnesses exist, how to launch them,
/// and where their configuration lives) and can be overridden or extended by
/// an optional `agents.toml`.
#[derive(Debug, Clone)]
pub struct AgentDef {
    /// Short, stable name used as the agent-type key and shown in the UI.
    pub name: String,
    /// Executable to launch.
    pub command: String,
    /// Human-readable description.
    pub description: String,
    /// Default arguments passed on launch.
    pub default_args: Vec<String>,
    /// Canonical `agent_manager` harness id, when this maps to a known harness.
    pub harness_id: Option<String>,
    /// Filesystem root of the harness configuration, when known.
    pub config_root: Option<String>,
}

impl AgentDef {
    /// Build a definition from a portable [`Harness`].
    fn from_harness(h: &dyn Harness) -> Self {
        Self {
            name: h.command().to_string(),
            command: h.command().to_string(),
            description: h.display_name().to_string(),
            default_args: Vec::new(),
            harness_id: Some(h.id()),
            config_root: None,
        }
    }
}

/// Raw TOML structure for a single agent override in `agents.toml`.
#[derive(Debug, Deserialize, Clone)]
struct RawAgentDef {
    name: String,
    command: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    default_args: Vec<String>,
}

/// Top-level TOML structure: a flat map of agent name → definition.
#[derive(Debug, Deserialize)]
struct AgentsConfig {
    #[serde(flatten)]
    agents: HashMap<String, RawAgentDef>,
}

/// Registry of all known agent types.
pub struct AgentRegistry {
    agents: HashMap<String, AgentDef>,
}

impl AgentRegistry {
    /// Registry seeded purely from the built-in `agent_manager` harnesses.
    pub fn builtin() -> Self {
        let agents = harness::all()
            .iter()
            .map(|h| {
                let def = AgentDef::from_harness(h.as_ref());
                (def.name.clone(), def)
            })
            .collect();
        Self { agents }
    }

    /// Load agent definitions: start from the built-in harness registry, then
    /// overlay any overrides from `agents.toml` at the given path.
    ///
    /// A missing file is not an error — the built-in registry is returned. A
    /// malformed file is an error.
    pub fn load(path: PathBuf) -> Result<Self, String> {
        let mut registry = Self::builtin();

        let contents = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                println!(
                    "No {} found; using {} built-in harness(es)",
                    path.display(),
                    registry.agents.len()
                );
                return Ok(registry);
            }
            Err(e) => return Err(format!("Failed to read {}: {}", path.display(), e)),
        };

        let config: AgentsConfig = toml::from_str(&contents)
            .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;

        for raw in config.agents.into_values() {
            registry.agents.insert(raw.name.clone(), Self::merge(raw));
        }

        println!(
            "Loaded {} agent type(s) ({} built-in + overrides from {})",
            registry.agents.len(),
            harness::all().len(),
            path.display()
        );

        Ok(registry)
    }

    /// Merge a raw `agents.toml` entry with portable harness knowledge.
    fn merge(raw: RawAgentDef) -> AgentDef {
        // Resolve harness knowledge by name or command (handles aliases too).
        let harness = harness::resolve(&raw.name).or_else(|| harness::resolve(&raw.command));

        let description = if raw.description.is_empty() {
            harness
                .as_ref()
                .map(|h| h.display_name().to_string())
                .unwrap_or_default()
        } else {
            raw.description
        };

        AgentDef {
            name: raw.name,
            command: raw.command,
            description,
            default_args: raw.default_args,
            harness_id: harness.as_ref().map(|h| h.id()),
            config_root: None,
        }
    }

    /// Get an agent definition by name.
    pub fn get(&self, name: &str) -> Option<&AgentDef> {
        self.agents.get(name)
    }

    /// List all agent types as info structs (for sending to the UI).
    pub fn list_all(&self) -> Vec<AgentTypeInfo> {
        self.agents
            .values()
            .map(|a| AgentTypeInfo {
                name: a.name.clone(),
                command: a.command.clone(),
                description: a.description.clone(),
                default_args: a.default_args.clone(),
                config_root: a.config_root.clone(),
            })
            .collect()
    }

    /// Check if an agent type exists.
    pub fn has(&self, name: &str) -> bool {
        self.agents.contains_key(name)
    }

    /// Create an empty registry (fallback when TOML loading fails).
    pub fn empty() -> Self {
        Self {
            agents: HashMap::new(),
        }
    }
}
