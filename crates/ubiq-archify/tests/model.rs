//! P1.3: the typed model reads every Archify example and every golden fixture that got past the
//! schema, and writes them back unchanged.

use std::fs;
use std::path::{Path, PathBuf};

use ubiq_archify::model::{self, Doc};
use serde_json::{Value, json};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn examples() -> Vec<(String, Value)> {
    let dir = crate_dir().join("tests/fixtures");
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if !path.to_string_lossy().ends_with(".golden.json") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let golden: Value = serde_json::from_str(&text).unwrap();
        if golden["group"] == "example" {
            let name = golden["source"].as_str().unwrap().rsplit('/').next().unwrap().to_owned();
            out.push((name, golden["source_doc"].clone()));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The goldens whose document reached the schema stage in Archify: it has a `source_doc` and none
/// of the three runs stopped at the CLI's input pre-checks or at a schema keyword.
fn goldens() -> Vec<(String, String, Value)> {
    let dir = crate_dir().join("tests/fixtures");
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).unwrap() {
        let path: &Path = &entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".golden.json") {
            continue;
        }
        let case: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        let Some(doc) = case.get("source_doc") else {
            continue;
        };
        let blocked = ["authored", "standard", "showcase"].iter().any(|run| {
            let diags = &case["validate_receipt"][run]["receipt"]["diagnostics"];
            diags.as_array().is_some_and(|ds| {
                ds.iter().any(|d| {
                    let code = d["code"].as_str().unwrap_or("");
                    ["schema/", "input/", "output/", "portable-path/"]
                        .iter()
                        .any(|p| code.starts_with(p))
                })
            })
        });
        if blocked {
            continue;
        }
        let ty = case["type"].as_str().expect("case type").to_owned();
        out.push((name, ty, doc.clone()));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Numbers compare as `f64` (the model writes `1.0` where the source has `1`) and `meta.views`
/// is dropped.
fn normalise(mut v: Value) -> Value {
    fn walk(v: &mut Value) {
        match v {
            Value::Number(n) => *v = json!(n.as_f64().unwrap()),
            Value::Array(a) => a.iter_mut().for_each(walk),
            Value::Object(o) => o.values_mut().for_each(walk),
            _ => {}
        }
    }
    if let Some(meta) = v.get_mut("meta").and_then(Value::as_object_mut) {
        meta.remove("views");
    }
    walk(&mut v);
    v
}

fn assert_round_trips(name: &str, ty: &str, source: &Value) {
    let doc = model::parse(ty, source).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(doc.diagram_type(), ty, "{name}");
    let back = serde_json::to_value(&doc).unwrap();
    assert_eq!(
        normalise(back),
        normalise(source.clone()),
        "{name}: round trip"
    );
}

#[test]
fn every_example_deserialises_and_round_trips() {
    let all = examples();
    assert_eq!(all.len(), 15, "examples/*.json");
    for (name, value) in &all {
        let ty = value["diagram_type"].as_str().unwrap();
        assert_round_trips(name, ty, value);
        let via_tag = Doc::from_value(value).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(via_tag.diagram_type(), ty);
        let via_serde: Doc = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(via_serde, via_tag, "{name}");
    }
}

#[test]
fn every_golden_past_the_schema_deserialises_and_round_trips() {
    let all = goldens();
    // The 36 positives plus the negatives that fail after the schema (149 today).
    assert!(all.len() > 100, "only {} goldens qualified", all.len());
    for (name, ty, doc) in &all {
        assert_round_trips(name, ty, doc);
    }
}

#[test]
fn views_are_accepted_and_dropped() {
    let mut doc = json!({
        "schema_version": 1, "diagram_type": "dataflow",
        "meta": {"title": "t", "output": "o.html",
                 "views": [{"id": "v", "label": "V", "focus": ["a"]}]},
        "stages": [{"label": "a"}, {"label": "b"}],
        "nodes": [], "flows": []
    });
    let parsed = model::parse("dataflow", &doc).unwrap();
    assert!(!serde_json::to_string(&parsed).unwrap().contains("views"));
    doc["meta"].as_object_mut().unwrap().remove("views");
    assert_eq!(parsed, model::parse("dataflow", &doc).unwrap());
}

#[test]
fn integer_fields_take_floats() {
    let doc = json!({
        "schema_version": 2.0, "diagram_type": "lifecycle",
        "meta": {"title": "t", "output": "o.html"},
        "lanes": [{"id": "l", "label": "L"}],
        "states": [{"id": "a", "type": "start", "label": "A", "lane": "l", "col": 1.0}],
        "transitions": []
    });
    let Doc::Lifecycle(l) = model::parse("lifecycle", &doc).unwrap() else {
        panic!()
    };
    assert_eq!(l.states[0].col, 1.0);
    assert_eq!(l.schema_version, model::common::SchemaVersion::V2);
}

#[test]
fn bad_input_is_an_error_never_a_panic() {
    let ok = json!({
        "schema_version": 1, "diagram_type": "sequence",
        "meta": {"title": "t", "output": "o.html"},
        "participants": [], "messages": []
    });
    assert!(model::parse("sequence", &ok).is_ok());
    // A type the caller did not ask for, an unknown type, an unknown field, a wrong shape.
    assert!(model::parse("workflow", &ok).is_err());
    assert!(model::parse("gantt", &ok).is_err());
    let mut extra = ok.clone();
    extra["surprise"] = json!(1);
    assert!(model::parse("sequence", &extra).is_err());
    let mut v2 = ok.clone();
    v2["schema_version"] = json!(2);
    assert!(model::parse("sequence", &v2).is_err());
    for garbage in [
        json!(null),
        json!(7),
        json!("x"),
        json!([]),
        json!({}),
        json!({"meta": []}),
    ] {
        for ty in [
            "architecture",
            "workflow",
            "sequence",
            "dataflow",
            "lifecycle",
            "",
        ] {
            assert!(model::parse(ty, &garbage).is_err());
        }
        assert!(Doc::from_value(&garbage).is_err());
    }
}
