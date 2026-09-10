//! Who is calling: what the host knows about each running agent, keyed by the string its URL
//! carries.
//!
//! The server is stateless — every request says who it is in its path and nothing is remembered
//! between two of them — so this is the one place identity comes from. The coordinator writes it
//! when a harness launches and takes the row out when the run is retired; the listener thread only
//! ever reads, under a read lock, because the coordinator's thread is the one thing in this
//! process that must never wait on a request being served.
//!
//! **Facts, never material.** An account is here by *id*, which is the account family's rule
//! everywhere else and is not weakened by the fact that the reader is an agent Ubiq started
//! itself: a tool call's answer is text a model may repeat anywhere.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Everything the built-in servers can say about one running agent.
///
/// A snapshot taken at launch rather than a handle back into the coordinator: the run's identity —
/// which account answered, which model was picked, which folder it works in — is fixed for the
/// life of the process it launched, and reading it must not need the coordinator's thread.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentFacts {
    /// The agent's own id, and the segment its URL carries: `AgentId` for a conversation, `PaneId`
    /// for a pane. The same string [`crate::agent::Agents`] names the run directory with.
    pub key: String,
    /// What the interface calls this agent — the name on its card.
    pub name: String,
    /// The harness behind it, as a person reads it.
    pub harness: String,
    /// The identity it runs as, by id. Never a credential.
    pub account: Option<String>,
    /// The model it launched with, where one was picked.
    pub model: Option<String>,
    /// The harness's own permission mode, where one was picked.
    pub mode: Option<String>,
    /// The folder it works in.
    pub cwd: String,
    /// The harness's own session id, when this run was resumed into an existing conversation.
    /// `None` on a fresh start: the harness mints one and never tells Ubiq.
    pub session: Option<String>,
    /// The project the agent was started in.
    pub project: ProjectFacts,
}

/// The project half of the same snapshot, as `project_info` answers it.
///
/// [`ubiq_proto::projects::ProjectRecord`] carries no description, so there is none here — a
/// field invented for an agent to read would be a field nothing writes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectFacts {
    pub id: String,
    pub name: String,
    pub path: String,
    /// The swatch index the interface drew it with. Passed through as the number it is: the host
    /// holds no palette and resolves no colour (`D2`).
    pub colour: usize,
}

/// Every agent the MCP servers will answer for, by URL segment.
///
/// Cloning shares the table — the coordinator keeps one clone and the listener thread another.
#[derive(Clone, Default)]
pub struct Registry(Arc<RwLock<HashMap<String, AgentFacts>>>);

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remember an agent, from the launch that started it. Replaces whatever was there: a relaunch
    /// of the same conversation is the same agent with fresher facts.
    pub fn register(&self, facts: AgentFacts) {
        self.write().insert(facts.key.clone(), facts);
    }

    /// Forget one, from the retirement that ended it. A key that is not there is not an error —
    /// every path that retires a run calls this, and only one of them launched it.
    pub fn forget(&self, key: &str) {
        self.write().remove(key);
    }

    /// What is known about the agent a URL named, or `None` — which the listener turns into a 404,
    /// because a request naming nobody is not a protocol error, it is the wrong address.
    pub fn facts(&self, key: &str) -> Option<AgentFacts> {
        self.read().get(key).cloned()
    }

    /// How many agents are registered. For tests and for the log line at startup.
    pub fn len(&self) -> usize {
        self.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A poisoned lock means a serving thread panicked mid-write. The table is a cache of facts,
    /// not an invariant anything else depends on, so it is taken back rather than propagated —
    /// refusing to answer forever because one request panicked once would be the worse failure.
    fn read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, AgentFacts>> {
        self.0
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, AgentFacts>> {
        self.0
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("agents", &self.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(key: &str) -> AgentFacts {
        AgentFacts {
            key: key.to_string(),
            name: "Agent 1".to_string(),
            harness: "Claude Code".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn a_registered_agent_is_found_and_a_forgotten_one_is_not() {
        let registry = Registry::new();
        registry.register(facts("abc"));
        assert_eq!(
            registry.facts("abc").map(|f| f.name),
            Some("Agent 1".into())
        );
        assert!(registry.facts("nobody").is_none());
        registry.forget("abc");
        assert!(registry.facts("abc").is_none());
        // Forgetting twice is what the retire paths actually do.
        registry.forget("abc");
        assert!(registry.is_empty());
    }
}
