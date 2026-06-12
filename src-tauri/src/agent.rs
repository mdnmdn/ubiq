use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use crate::messages::AgentTypeInfo;

/// Raw TOML structure for a single agent definition.
#[derive(Debug, Deserialize, Clone)]
pub struct AgentDef {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub default_args: Vec<String>,
}

/// Top-level TOML structure: a flat map of agent name → definition.
#[derive(Debug, Deserialize)]
pub struct AgentsConfig {
    #[serde(flatten)]
    pub agents: HashMap<String, AgentDef>,
}

/// Registry of all known agent types, loaded from TOML at startup.
pub struct AgentRegistry {
    agents: HashMap<String, AgentDef>,
}

impl AgentRegistry {
    /// Load agent definitions from `agents.toml` at the given path.
    pub fn load(path: PathBuf) -> Result<Self, String> {
        let contents = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        let config: AgentsConfig = toml::from_str(&contents)
            .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;

        println!(
            "Loaded {} agent type(s) from {}",
            config.agents.len(),
            path.display()
        );

        Ok(Self {
            agents: config.agents,
        })
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
