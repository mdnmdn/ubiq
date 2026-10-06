//! From an interpreter error to a wire diagnostic: path, nearest identity, message, evidence.
//! `renderers/shared/validator.mjs` (`annotatedPath`, `annotatePath`, `validateSchema`).

use serde_json::{Map, Value};

use super::fixes;
use super::interp::SchemaError;
use crate::diag::{Diagnostic, Subject};

/// `subject.path` and `subject.identity` of an instance path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotated {
    /// The instance path, `/` for the root.
    pub path: String,
    /// The nearest enclosing object's `id`, else `label`, found while walking the path.
    pub identity: Option<String>,
}

/// `annotatedPath`. The walk does not unescape `~0`/`~1`, as in Archify: a key containing them
/// stops the walk with the identity found so far.
pub fn annotated_path(instance_path: &str, data: &Value) -> Annotated {
    if instance_path.is_empty() {
        return Annotated {
            path: "/".into(),
            identity: None,
        };
    }
    let mut node = Some(data);
    let mut hint = None;
    for segment in instance_path.split('/').skip(1) {
        let Some(current @ (Value::Object(_) | Value::Array(_))) = node else {
            break;
        };
        node = match current {
            Value::Array(items) => segment
                .bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| segment.parse::<usize>().ok())
                .flatten()
                .and_then(|i| items.get(i)),
            Value::Object(map) => map.get(segment),
            _ => None,
        };
        if let Some(Value::Object(map)) = node {
            let tag = map
                .get("id")
                .filter(|v| !v.is_null())
                .or_else(|| map.get("label"))
                .filter(|v| !v.is_null());
            if let Some(tag) = tag {
                hint = Some(js_string(tag));
            }
        }
    }
    Annotated {
        path: instance_path.to_owned(),
        identity: hint,
    }
}

/// `String(value)`.
fn js_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        Value::Array(items) => items
            .iter()
            .map(|v| {
                if v.is_null() {
                    String::new()
                } else {
                    js_string(v)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
        other => other.to_string(),
    }
}

/// `annotatePath`: `/nodes/3 (id/label: "router")`.
pub fn annotate_path(annotated: &Annotated) -> String {
    match &annotated.identity {
        Some(identity) => format!(
            "{} (id/label: {})",
            annotated.path,
            serde_json::to_string(identity).unwrap_or_default()
        ),
        None => annotated.path.clone(),
    }
}

/// `JSON.stringify(params)`, keys in the order ajv built them.
fn params_json(error: &SchemaError) -> String {
    let fields: Vec<String> = error
        .params
        .iter()
        .map(|(key, value)| {
            format!(
                "{}:{}",
                serde_json::to_string(key).unwrap_or_default(),
                value
            )
        })
        .collect();
    format!("{{{}}}", fields.join(","))
}

/// One interpreter error as the diagnostic `validateSchema` builds.
pub fn to_diagnostic(doc_type: &str, error: &SchemaError, data: &Value) -> Diagnostic {
    let annotated = annotated_path(&error.instance_path, data);
    let detail = if error.params.is_empty() {
        String::new()
    } else {
        format!(" {}", params_json(error))
    };
    let message = format!("{} {}{}", annotate_path(&annotated), error.message, detail);

    let mut evidence = Map::new();
    evidence.insert("keyword".into(), error.keyword.into());
    for (key, value) in &error.params {
        evidence.insert((*key).into(), value.clone());
    }

    let code = format!("schema/{}", error.keyword);
    let mut subject = Subject::of(doc_type)
        .with_path(annotated.path.clone())
        .with_rule(code.clone());
    subject.identity = annotated.identity.clone();
    Diagnostic::error(&code, &message)
        .with_subject(subject)
        .with_evidence(evidence)
        .with_fixes(fixes::supported_fixes(error, &annotated.path))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn identity_is_the_nearest_id_else_label() {
        let doc = json!({"nodes": [{"id": "a"}, {"label": "L", "meta": {"x": 1}}, {"id": "r", "label": "ignored", "k": {"label": "inner"}}]});
        let at = |p: &str| annotated_path(p, &doc);
        assert_eq!(
            at(""),
            Annotated {
                path: "/".into(),
                identity: None
            }
        );
        assert_eq!(
            at("/nodes"),
            Annotated {
                path: "/nodes".into(),
                identity: None
            }
        );
        assert_eq!(at("/nodes/0").identity.as_deref(), Some("a"));
        assert_eq!(at("/nodes/1").identity.as_deref(), Some("L"));
        // Walks in: the inner label wins over the outer id; one without a tag keeps the outer.
        assert_eq!(at("/nodes/1/meta/x").identity.as_deref(), Some("L"));
        assert_eq!(at("/nodes/2/k").identity.as_deref(), Some("inner"));
        // A missing child stops the walk.
        assert_eq!(at("/nodes/9/id").identity, None);
        assert_eq!(at("/nodes/0/missing/deeper").identity.as_deref(), Some("a"));
    }

    #[test]
    fn identity_stringifies_like_js() {
        let doc = json!({"a": [{"id": 7}, {"id": null, "label": "L"}, {"id": 1.5}, {"id": true}, {"id": [1, null, "x"]}]});
        let id = |i: usize| annotated_path(&format!("/a/{i}"), &doc).identity;
        assert_eq!(id(0).as_deref(), Some("7"));
        assert_eq!(id(1).as_deref(), Some("L"));
        assert_eq!(id(2).as_deref(), Some("1.5"));
        assert_eq!(id(3).as_deref(), Some("true"));
        assert_eq!(id(4).as_deref(), Some("1,,x"));
    }

    #[test]
    fn message_and_evidence_follow_validator_mjs() {
        let doc = json!({"nodes": [{"id": "router", "colour": "red"}]});
        let error = SchemaError {
            keyword: "additionalProperties",
            instance_path: "/nodes/0".into(),
            message: "must NOT have additional properties".into(),
            params: vec![("additionalProperty", json!("colour"))],
            schema: Some(json!(false)),
        };
        let d = to_diagnostic("workflow", &error, &doc);
        assert_eq!(d.code, "schema/additionalProperties");
        assert_eq!(
            d.message,
            "/nodes/0 (id/label: \"router\") must NOT have additional properties {\"additionalProperty\":\"colour\"}"
        );
        assert_eq!(d.subject.path.as_deref(), Some("/nodes/0"));
        assert_eq!(d.subject.identity.as_deref(), Some("router"));
        assert_eq!(d.subject.diagram_type.as_deref(), Some("workflow"));
        assert_eq!(
            d.subject.rule.as_deref(),
            Some("schema/additionalProperties")
        );
        assert_eq!(
            Value::Object(d.evidence.clone()),
            json!({"keyword": "additionalProperties", "additionalProperty": "colour"})
        );
        assert_eq!(
            d.supported_fixes,
            ["remove unsupported property \"colour\""]
        );
        assert!(!d.supported_fixes.is_empty() && d.is_error());
    }
}
