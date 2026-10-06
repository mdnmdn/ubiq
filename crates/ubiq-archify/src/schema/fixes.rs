//! `supportedFixes` for schema errors: the 11 keywords `validator.mjs` generates a fix for
//! (00 §3.2). Every other keyword gets none.

use serde_json::Value;

use super::interp::SchemaError;

/// `JSON.stringify(value)`; `undefined` when the param is absent.
fn json(value: Option<&Value>) -> String {
    value.map_or_else(|| "undefined".into(), Value::to_string)
}

/// A number or string the way a JS template literal prints it.
fn plain(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => "undefined".into(),
    }
}

pub fn supported_fixes(error: &SchemaError, path: &str) -> Vec<String> {
    let p = |key: &str| error.param(key);
    let comparison = |default: &str| match p("comparison") {
        Some(Value::String(c)) if !c.is_empty() => c.clone(),
        _ => default.to_owned(),
    };
    let fix = match error.keyword {
        "additionalProperties" => {
            format!(
                "remove unsupported property {}",
                json(p("additionalProperty"))
            )
        }
        "required" => format!("add required property {}", json(p("missingProperty"))),
        "type" => format!("use {} at {path}", json(p("type"))),
        "enum" => format!(
            "choose one of {}",
            p("allowedValues").map_or_else(|| "[]".into(), Value::to_string)
        ),
        "pattern" => format!("match the required pattern {}", json(p("pattern"))),
        "minimum" => format!("use a value {} {}", comparison(">="), plain(p("limit"))),
        "maximum" => format!("use a value {} {}", comparison("<="), plain(p("limit"))),
        "minItems" => format!("provide at least {} item(s)", plain(p("limit"))),
        "maxItems" => format!("provide at most {} item(s)", plain(p("limit"))),
        "minLength" => format!("provide at least {} character(s)", plain(p("limit"))),
        "maxLength" => format!("provide at most {} character(s)", plain(p("limit"))),
        _ => return Vec::new(),
    };
    vec![fix]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn error(keyword: &'static str, params: Vec<(&'static str, Value)>) -> SchemaError {
        SchemaError {
            keyword,
            instance_path: "/x".into(),
            message: String::new(),
            params,
            schema: None,
        }
    }

    #[test]
    fn the_eleven_keywords() {
        let fixes = |kw, params| supported_fixes(&error(kw, params), "/x");
        assert_eq!(
            fixes(
                "additionalProperties",
                vec![("additionalProperty", json!("c"))]
            ),
            ["remove unsupported property \"c\""]
        );
        assert_eq!(
            fixes("required", vec![("missingProperty", json!("id"))]),
            ["add required property \"id\""]
        );
        assert_eq!(
            fixes("type", vec![("type", json!("string"))]),
            ["use \"string\" at /x"]
        );
        assert_eq!(
            fixes("enum", vec![("allowedValues", json!(["a", 1]))]),
            ["choose one of [\"a\",1]"]
        );
        assert_eq!(
            fixes("pattern", vec![("pattern", json!("^a$"))]),
            ["match the required pattern \"^a$\""]
        );
        assert_eq!(
            fixes(
                "minimum",
                vec![("comparison", json!(">=")), ("limit", json!(0.5))]
            ),
            ["use a value >= 0.5"]
        );
        assert_eq!(
            fixes(
                "maximum",
                vec![("comparison", json!("<=")), ("limit", json!(5))]
            ),
            ["use a value <= 5"]
        );
        assert_eq!(
            fixes("minItems", vec![("limit", json!(2))]),
            ["provide at least 2 item(s)"]
        );
        assert_eq!(
            fixes("maxItems", vec![("limit", json!(3))]),
            ["provide at most 3 item(s)"]
        );
        assert_eq!(
            fixes("minLength", vec![("limit", json!(1))]),
            ["provide at least 1 character(s)"]
        );
        assert_eq!(
            fixes("maxLength", vec![("limit", json!(80))]),
            ["provide at most 80 character(s)"]
        );
    }

    #[test]
    fn other_keywords_have_none() {
        for kw in [
            "const",
            "exclusiveMinimum",
            "oneOf",
            "anyOf",
            "not",
            "items",
            "propertyNames",
            "minProperties",
            "portablePath",
        ] {
            assert!(supported_fixes(&error(kw, vec![]), "/x").is_empty(), "{kw}");
        }
    }
}
