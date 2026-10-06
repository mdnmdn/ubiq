//! Schema stage (V3): the six Archify schemas embedded verbatim, a mini interpreter for the
//! keywords they use, and the portable-path wrapper (P1.2, D6, D8).
//!
//! `validate(doc_type, &doc)` returns every `schema/<keyword>` failure with Archify's code,
//! JSON-Pointer path and nearest-identity (00 §3.2, 01 §3.2, §3.3). Nothing else of the pipeline
//! lives here: V1/V2 (read, parse, `meta.output`) and V4 onwards are `compile.rs`'s.

pub mod annotate;
pub mod fixes;
pub mod interp;
pub mod portable_path;

use std::sync::OnceLock;

use serde_json::Value;

use crate::diag::{Diagnostic, Subject};
use interp::{Registry, SchemaError};

/// The five diagram types, in Archify's order.
pub const DOC_TYPES: [&str; 5] = [
    "architecture",
    "workflow",
    "sequence",
    "dataflow",
    "lifecycle",
];

const COMMON: &str = include_str!("schemas/common.schema.json");
const ARCHITECTURE: &str = include_str!("schemas/architecture.schema.json");
const WORKFLOW: &str = include_str!("schemas/workflow.schema.json");
const SEQUENCE: &str = include_str!("schemas/sequence.schema.json");
const DATAFLOW: &str = include_str!("schemas/dataflow.schema.json");
const LIFECYCLE: &str = include_str!("schemas/lifecycle.schema.json");

/// The verbatim schema text of a diagram type (what `archify_schema` serves), or `common`.
pub fn source(name: &str) -> Option<&'static str> {
    match name {
        "common" => Some(COMMON),
        "architecture" => Some(ARCHITECTURE),
        "workflow" => Some(WORKFLOW),
        "sequence" => Some(SEQUENCE),
        "dataflow" => Some(DATAFLOW),
        "lifecycle" => Some(LIFECYCLE),
        _ => None,
    }
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Registry::new(&[
            COMMON,
            ARCHITECTURE,
            WORKFLOW,
            SEQUENCE,
            DATAFLOW,
            LIFECYCLE,
        ])
    })
}

fn schema_id(doc_type: &str) -> String {
    format!("https://github.com/tt-a1i/archify/schemas/{doc_type}.schema.json")
}

/// The raw ajv-shaped errors, before they become diagnostics. The `portablePath` wrapper runs only
/// when the schema passed, as in Archify's generated validators. `None` for an unknown type.
pub fn errors(doc_type: &str, doc: &Value) -> Option<Vec<SchemaError>> {
    if !DOC_TYPES.contains(&doc_type) {
        return None;
    }
    let mut errors = registry().validate(&schema_id(doc_type), doc);
    if errors.is_empty()
        && let Some(output) = doc.pointer("/meta/output").and_then(Value::as_str)
        && let Err(error) = portable_path::validate(output)
    {
        errors.push(SchemaError {
            keyword: "portablePath",
            instance_path: "/meta/output".into(),
            message: "must satisfy the portable output path contract".into(),
            params: vec![("reason", error.reason.into())],
            schema: None,
        });
    }
    Some(errors)
}

/// Stage V3: every schema failure of `doc` against the `doc_type` schema, in Archify's order.
/// An unknown type is a single `cli/unknown-diagram-type`.
pub fn validate(doc_type: &str, doc: &Value) -> Vec<Diagnostic> {
    match errors(doc_type, doc) {
        Some(list) => list
            .iter()
            .map(|error| annotate::to_diagnostic(doc_type, error, doc))
            .collect(),
        None => vec![
            Diagnostic::error(
                "cli/unknown-diagram-type",
                &format!("Unknown diagram type \"{doc_type}\"."),
            )
            .with_subject(Subject::default().with_extra("type", doc_type.into()))
            .with_fixes([format!("use one of {}", DOC_TYPES.join(", "))]),
        ],
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn every_embedded_pattern_compiles() {
        // `Registry` ignores a pattern it cannot compile, which would silently pass bad input.
        fn walk(value: &Value, patterns: &mut Vec<String>) {
            match value {
                Value::Object(map) => {
                    for (key, child) in map {
                        if key == "pattern"
                            && let Some(p) = child.as_str()
                        {
                            patterns.push(p.to_owned());
                        }
                        walk(child, patterns);
                    }
                }
                Value::Array(items) => items.iter().for_each(|i| walk(i, patterns)),
                _ => {}
            }
        }
        let mut patterns = Vec::new();
        for name in [
            "common",
            "architecture",
            "workflow",
            "sequence",
            "dataflow",
            "lifecycle",
        ] {
            let doc: Value = serde_json::from_str(source(name).unwrap()).unwrap();
            walk(&doc, &mut patterns);
        }
        assert!(patterns.len() >= 15, "{} patterns", patterns.len());
        for p in patterns {
            assert!(interp::compile_js_pattern(&p).is_some(), "{p}");
        }
    }

    #[test]
    fn the_six_schemas_are_the_ones_archify_ships() {
        for name in [
            "common",
            "architecture",
            "workflow",
            "sequence",
            "dataflow",
            "lifecycle",
        ] {
            let doc: Value = serde_json::from_str(source(name).unwrap()).unwrap();
            assert_eq!(
                doc["$id"],
                json!(format!(
                    "https://github.com/tt-a1i/archify/schemas/{name}.schema.json"
                ))
            );
        }
        assert!(source("nope").is_none());
    }

    #[test]
    fn unknown_type_is_one_diagnostic() {
        let d = validate("gantt", &json!({}));
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "cli/unknown-diagram-type");
    }

    #[test]
    fn the_portable_path_wrapper_runs_only_after_a_clean_schema() {
        let doc = |output: &str| json!({"schema_version": 1, "diagram_type": "dataflow", "meta": {"title": "t", "output": output}});
        // The schema's own patterns reject this one first; the wrapper never speaks.
        assert!(
            validate("dataflow", &doc("a.htm"))
                .iter()
                .all(|d| d.code != "schema/portablePath")
        );
        // 100 euro signs: 100 characters, 300 UTF-8 bytes. The regexes pass it; the contract does not.
        let long = format!("{}.html", "\u{20ac}".repeat(100));
        let schema_only = registry().validate(&schema_id("dataflow"), &doc(&long));
        assert!(
            schema_only
                .iter()
                .all(|e| e.instance_path != "/meta/output")
        );
        let d = validate(
            "dataflow",
            &json!({"schema_version": 1, "diagram_type": "dataflow",
            "meta": {"title": "t", "output": long},
            "stages": [{"label": "A"}, {"label": "B"}],
            "nodes": [
                {"id": "a", "type": "backend", "label": "A", "stage": 0, "row": 0},
                {"id": "b", "type": "backend", "label": "B", "stage": 1, "row": 0}
            ],
            "flows": [{"from": "a", "to": "b", "label": "f"}]}),
        );
        let wrapper: Vec<_> = d
            .iter()
            .filter(|d| d.code == "schema/portablePath")
            .collect();
        assert_eq!(wrapper.len(), 1, "{d:#?}");
        let w = wrapper[0];
        assert_eq!(w.subject.path.as_deref(), Some("/meta/output"));
        assert_eq!(w.evidence["reason"], "component-too-long");
        assert_eq!(
            w.message,
            "/meta/output must satisfy the portable output path contract {\"reason\":\"component-too-long\"}"
        );
    }
}
