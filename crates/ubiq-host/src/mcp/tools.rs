//! What the built-in tools actually do.
//!
//! Every one of them is answered on the listener's own thread, from the [`AgentFacts`] the URL
//! resolved to and nothing else. That is the whole reason the server can be stateless: a tool here
//! never asks the coordinator a question, it either reports a fact taken at launch or files
//! something and returns.
//!
//! **Filing is fire-and-forget.** `send_notification` says [`Message::RaiseNotification`] through
//! a [`Voice`] and answers immediately — the notification centre decides the mute rules and the
//! broadcast on the coordinator's thread, exactly as it does for a window, and a harness waiting
//! on a round trip through that thread would be a harness a busy coordinator can stall.
//!
//! An `Err` here is not a JSON-RPC error: the caller turns it into MCP's in-band `isError`, which
//! is what a model can read and correct. See [`super::server::dispatch`].

use serde_json::{Value, json};
use ubiq_proto::bus::Voice;
use ubiq_proto::messages::Message;
use ubiq_proto::notifications::{Family, Level, NotificationRequest};

use super::catalogue::{PROJECT_INFO, TEST};
use super::registry::AgentFacts;

/// Call one tool. `server` and `tool` have already been matched against the catalogue's server;
/// the tool has not, so an unknown one ends here as the in-band error a model sees.
pub fn call(
    server: &str,
    tool: &str,
    arguments: &Value,
    facts: &AgentFacts,
    voice: &Voice,
) -> Result<Value, String> {
    match (server, tool) {
        (TEST, "send_notification") => send_notification(arguments, facts, voice),
        (TEST, "write_log") => write_log(arguments, facts),
        (PROJECT_INFO, "project_info") => Ok(project_info(facts)),
        (PROJECT_INFO, "whoami") => Ok(whoami(facts)),
        _ => Err(format!("unknown tool: {server}/{tool}")),
    }
}

/// Raise a real Ubiq notification, attributed to the agent that asked for it.
///
/// The actor is the agent's own name and the category its harness, so the bell reads as the
/// conversation talking and a mute rule can silence one agent — or one harness — without silencing
/// the family. The family is [`Family::Agents`] because that is what this is: a harness said
/// something.
fn send_notification(
    arguments: &Value,
    facts: &AgentFacts,
    voice: &Voice,
) -> Result<Value, String> {
    let text = arguments
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| "send_notification needs a non-empty 'text' argument".to_string())?;

    let level = match arguments.get("level").and_then(Value::as_str) {
        None | Some("info") => Level::Info,
        Some("warning") => Level::Warning,
        Some("error") => Level::Error,
        Some(other) => {
            return Err(format!(
                "unknown level '{other}': use one of info, warning, error"
            ));
        }
    };

    let request = NotificationRequest::new(level, Family::Agents, text)
        .with_actor(facts.name.clone())
        .with_category(facts.harness.clone());
    voice.say(Message::RaiseNotification { request });

    Ok(json!({"raised": true, "level": level.as_str(), "text": text}))
}

/// Write one line into Ubiq's own log ring.
///
/// Through `tracing::` from inside this module on purpose: the sink classifies by target, so a
/// line written here lands under `Subsystem::Mcp` and shows up in the log viewer's own filter
/// without anything being told about it (`crates/ubiq-proto/src/log.rs`). The agent's id is a
/// field rather than part of the text so a search for one agent finds every line it wrote.
fn write_log(arguments: &Value, facts: &AgentFacts) -> Result<Value, String> {
    let message = arguments
        .get("message")
        .and_then(Value::as_str)
        .filter(|message| !message.trim().is_empty())
        .ok_or_else(|| "write_log needs a non-empty 'message' argument".to_string())?;

    let agent = facts.key.as_str();
    match arguments.get("level").and_then(Value::as_str) {
        Some("debug") => tracing::debug!(agent, "{message}"),
        None | Some("info") => tracing::info!(agent, "{message}"),
        Some("warn") => tracing::warn!(agent, "{message}"),
        Some("error") => tracing::error!(agent, "{message}"),
        Some(other) => {
            return Err(format!(
                "unknown level '{other}': use one of debug, info, warn, error"
            ));
        }
    }

    Ok(json!({"logged": true, "message": message}))
}

/// The project this agent was started in. No description field: the record has none.
fn project_info(facts: &AgentFacts) -> Value {
    json!({
        "id": facts.project.id,
        "name": facts.project.name,
        "path": facts.project.path,
        "colour": facts.project.colour,
    })
}

/// What this agent is. The account by id and nothing beside it — the rule holds here as it holds
/// on the wire.
fn whoami(facts: &AgentFacts) -> Value {
    json!({
        "agent_id": facts.key,
        "name": facts.name,
        "harness": facts.harness,
        "account": facts.account,
        "model": facts.model,
        "permission_mode": facts.mode,
        "session_id": facts.session,
        "cwd": facts.cwd,
    })
}
