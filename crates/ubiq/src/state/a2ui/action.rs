//! What a click on a drawn surface produces.
//!
//! **An action is the only way anything a user did reaches the agent.** A2UI's inputs write into
//! the renderer's own copy of the data model and nothing else; the model crosses the boundary at
//! exactly one moment, which is when an action fires and the pointers named in its `context` are
//! resolved. That asymmetry is the protocol's, not Ubiq's, and it is why this module builds a
//! message rather than calling anything.
//!
//! **Nothing here performs.** An action carrying a function call — `openUrl` is the one the
//! catalog ships — is reported, never run. A payload is agent-authored text: it may not move this
//! window, open a browser or reach the network, and the place that rule is kept is the place the
//! call would otherwise have been made. See `D115`.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::{Deserializer, Error as _};
use serde_json::{Map, Value};

use super::{CheckRule, Dynamic, Parsed, Scope, eval, scoped};

/// A component's `action`: a message for the agent, or a function for the renderer.
///
/// The schema's own `oneOf`, kept open at the end. A shape neither arm recognises is carried
/// rather than dropped, so a button authored against a later version of the protocol draws as a
/// button that says it was not understood instead of as one that silently does nothing.
#[derive(Debug, Clone)]
pub enum Action {
    Event(Event),
    Call(eval::Call),
    Unknown(Value),
}

/// An event dispatched to the agent: what happened, and which values it happened to.
///
/// `context` is the payload of the whole protocol. Its values are literals or bindings, and the
/// bindings are resolved against the scope of the component that fired — so a button inside a
/// templated row sends that row's values rather than the first row's.
#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub name: String,
    #[serde(default, rename = "userMessage")]
    pub user_message: Option<Dynamic>,
    #[serde(default)]
    pub context: BTreeMap<String, Dynamic>,
}

impl<'de> Deserialize<'de> for Action {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let Value::Object(map) = &value else {
            return Ok(Action::Unknown(value));
        };
        if let Some(event) = map.get("event") {
            return serde_json::from_value(event.clone())
                .map(Action::Event)
                .map_err(D::Error::custom);
        }
        if let Some(call) = map.get("functionCall") {
            return serde_json::from_value(call.clone())
                .map(Action::Call)
                .map_err(D::Error::custom);
        }
        Ok(Action::Unknown(value))
    }
}

/// The renderer-to-agent message one fired event becomes.
///
/// `timestamp` is passed in rather than read from the clock here, so that a test can assert on the
/// whole message rather than on the shape of it with a hole in the middle.
///
/// The data model rides as a **sibling** of `action` rather than inside it. The specification says
/// a surface created with `sendDataModel` has the model attached to every message the renderer
/// sends, without saying where; a sibling is the reading that leaves the `action` object exactly
/// as its own schema describes it.
pub fn envelope(
    surface_id: &str,
    source_component_id: &str,
    event: &Event,
    model: &Value,
    scope: Scope<'_>,
    timestamp: &str,
    send_data_model: bool,
) -> Value {
    let mut action = Map::new();
    action.insert("name".into(), Value::String(event.name.clone()));
    action.insert("surfaceId".into(), Value::String(surface_id.to_string()));
    action.insert(
        "sourceComponentId".into(),
        Value::String(source_component_id.to_string()),
    );
    action.insert("timestamp".into(), Value::String(timestamp.to_string()));

    let context: Map<String, Value> = event
        .context
        .iter()
        .map(|(key, binding)| (key.clone(), binding.eval(model, scope)))
        .collect();
    action.insert("context".into(), Value::Object(context));

    if let Some(message) = &event.user_message {
        action.insert(
            "userMessage".into(),
            Value::String(message.as_display_string(model, scope)),
        );
    }

    let mut envelope = Map::new();
    envelope.insert("version".into(), Value::String("v1.0".into()));
    envelope.insert("action".into(), Value::Object(action));
    if send_data_model {
        envelope.insert("dataModel".into(), model.clone());
    }
    Value::Object(envelope)
}

/// Every check in the surface that does not pass, as the component's scoped key and what to say.
///
/// **A failing check disables the action rather than firing it**, which is the catalog's own
/// instruction, and it is why this is a property of the whole surface rather than of the button:
/// the rules live on the inputs, and the button is only where the consequence shows. The message
/// is the validation result's own, then the rule's fallback, then a last resort — an agent that
/// wrote neither still gets a line rather than a mystery.
pub fn blocking_checks(parsed: &Parsed) -> Vec<(String, String)> {
    let mut failures = Vec::new();
    super::walk(parsed, &mut |component, scope| {
        for rule in component.kind.checks() {
            let (valid, message) = eval::validation(&rule.condition.eval(&parsed.model, scope));
            if !valid {
                failures.push((
                    scoped(&component.id, scope),
                    message
                        .or_else(|| rule.message.clone())
                        .unwrap_or_else(|| "check failed".to_string()),
                ));
            }
        }
    });
    failures
}

/// What a check rule says when it fails, before anything is evaluated — for a renderer that wants
/// to show the rule beside the input rather than only after a click.
pub fn rule_message(rule: &CheckRule) -> Option<&str> {
    rule.message.as_deref()
}
