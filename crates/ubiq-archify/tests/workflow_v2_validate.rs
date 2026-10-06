//! P6.4: the whole workflow v2 compile (`gates::workflow_v2::compile`: prologue, solver, router,
//! `validateWorkflow`, canvas) against Archify's own `compileWorkflow` (D5), with no tolerance.
//!
//! `tests/workflow_v2_validate_cases.json` (`gen-workflow-v2-validate-cases.mjs`) holds the 550
//! route-corpus documents compiled as authored and under `--quality showcase`. A laid-out run must
//! agree on the receipt (canvas, columns, node rects, edge points, labels, warnings); a rejected run
//! on the error text and every diagnostic the compiler threw, fixes included.

use std::fs;
use std::path::PathBuf;

use ubiq_archify::diag::Diagnostic;
use ubiq_archify::gates::{Gate, workflow_v2};
use ubiq_archify::model::workflow::Workflow;
use serde_json::Value;

/// Quote every non-integer number outside a string so `serde_json` keeps the printed digits.
fn quote_floats(text: &str) -> String {
    let (bytes, mut out, mut i, mut in_string) = (text.as_bytes(), String::with_capacity(text.len() + 65536), 0, false);
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if c == b'\\' {
                out.push(c as char);
                i += 1;
            } else if c == b'"' {
                in_string = false;
            }
            let width = text[i..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&text[i..i + width]);
            i += width;
            continue;
        }
        if c == b'"' {
            in_string = true;
        }
        if c == b'-' || c.is_ascii_digit() {
            let end = (i..bytes.len()).find(|&j| !matches!(bytes[j], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')).unwrap_or(bytes.len());
            let token = &text[i..end];
            if token.contains(['.', 'e', 'E']) {
                out.push_str(&format!("\"{token}\""));
            } else {
                out.push_str(token);
            }
            i = end;
            continue;
        }
        out.push(c as char);
        i += 1;
    }
    out
}

/// `got` (a number as a JSON value) against the quoted expectation.
fn same(got: &Value, want: &Value) -> bool {
    match (got, want) {
        (Value::Number(g), Value::String(w)) => w.parse::<f64>().is_ok_and(|w| g.as_f64() == Some(w)),
        (Value::Number(g), Value::Number(w)) => g.as_f64() == w.as_f64(),
        (Value::Array(g), Value::Array(w)) => g.len() == w.len() && g.iter().zip(w).all(|(a, b)| same(a, b)),
        (Value::Object(g), Value::Object(w)) => g.len() == w.len() && g.iter().all(|(k, v)| w.get(k).is_some_and(|x| same(v, x))),
        _ => got == want,
    }
}

/// A diagnostic as Archify prints it: no `subject.rule`, no `suppresses`.
fn json_of(d: &Diagnostic) -> Value {
    let mut v = serde_json::to_value(d).unwrap();
    if let Some(s) = v["subject"].as_object_mut() {
        s.remove("rule");
    }
    if let Some(o) = v.as_object_mut() {
        o.remove("suppresses");
    }
    v
}

fn points(edges: &[ubiq_archify::route::workflow::RoutedEdge]) -> Value {
    Value::Array(edges.iter().map(|e| Value::Array(e.points.iter().map(|p| serde_json::json!([p[0], p[1]])).collect())).collect())
}

#[test]
fn the_v2_compile_matches_archifys_compiler() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/workflow_v2_validate_cases.json");
    let text = fs::read_to_string(path).unwrap();
    let plain: Vec<Value> = serde_json::from_str(&text).unwrap();
    let quoted: Vec<Value> = serde_json::from_str(&quote_floats(&text)).unwrap();
    let (mut runs, mut clean, mut rejected, mut warned, mut report) = (0, 0, 0, 0, Vec::<String>::new());
    for (case, want_case) in plain.iter().zip(&quoted) {
        let name = case["name"].as_str().unwrap();
        let w: Workflow = serde_json::from_value(case["doc"].clone()).expect("a valid workflow");
        for (run, gate) in [("authored", workflow_v2::gate_of(&w)), ("showcase", Gate(Some("showcase")))] {
            let want = &want_case[run];
            if want["crashed"] == true {
                continue;
            }
            runs += 1;
            let mut problems = Vec::<String>::new();
            match (workflow_v2::compile(&w, gate, true), want["ok"] == true) {
                (Ok(g), true) => {
                    clean += 1;
                    let r = &want["receipt"];
                    let ours = serde_json::json!({
                        "viewBox": g.view_box, "requiredViewBox": g.required_view_box, "columns": g.placement.layout.col_xs,
                        "nodes": g.placement.nodes.iter().map(|n| serde_json::json!([n.rect.x, n.rect.y, n.rect.width, n.rect.height])).collect::<Vec<_>>(),
                        "points": points(&g.edges),
                        "labels": g.labels.iter().map(|l| serde_json::json!([l.text, l.at[0], l.at[1], l.rect.width])).collect::<Vec<_>>(),
                    });
                    let theirs = serde_json::json!({
                        "viewBox": r["viewBox"], "requiredViewBox": r["requiredViewBox"], "columns": r["columns"],
                        "nodes": r["nodes"].as_array().unwrap().iter().map(|n| serde_json::json!([n["x"], n["y"], n["width"], n["height"]])).collect::<Vec<_>>(),
                        "points": Value::Array(r["edges"].as_array().unwrap().iter().map(|e| e["points"].clone()).collect()),
                        "labels": r["labels"].as_array().unwrap().iter().map(|l| serde_json::json!([l["label"], l["x"], l["y"], l["width"]])).collect::<Vec<_>>(),
                    });
                    if !same(&ours, &theirs) {
                        problems.push(format!("receipt\n    got  {ours}\n    want {theirs}"));
                    }
                    let warnings: Vec<Value> = g.diagnostics.iter().map(json_of).collect();
                    warned += usize::from(!warnings.is_empty());
                    if !same(&Value::Array(warnings.clone()), &r["diagnostics"]) {
                        problems.push(format!("warnings\n    got  {}\n    want {}", Value::Array(warnings), r["diagnostics"]));
                    }
                }
                (Err(f), false) => {
                    rejected += 1;
                    if f.error != want["error"].as_str().unwrap_or_default() {
                        problems.push(format!("error\n    got  {:?}\n    want {:?}", f.error, want["error"]));
                    }
                    let ours = Value::Array(f.diagnostics.iter().map(json_of).collect());
                    if !same(&ours, &want["diagnostics"]) {
                        problems.push(format!("diagnostics\n    got  {ours}\n    want {}", want["diagnostics"]));
                    }
                }
                (Ok(_), false) => problems.push(format!("we lay it out; Archify failed: {}", want["error"])),
                (Err(f), true) => problems.push(format!("we fail ({}); Archify laid it out", f.error)),
            }
            if !problems.is_empty() {
                report.push(format!("{name} [{run}]\n  {}", problems.join("\n  ")));
            }
        }
    }
    println!("{} documents, {runs} runs: {clean} laid out ({warned} with warnings), {rejected} rejected, {} differ", plain.len(), report.len());
    assert!(clean > 300 && rejected > 300, "only {clean} clean and {rejected} rejected runs");
    assert!(report.is_empty(), "{} runs differ:\n{}", report.len(), report.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}
