//! P6.3: the workflow v2 router against Archify (D5), with no tolerance (`==` on `f64`).
//!
//! - Every `readable-v2` golden that laid out: the receipt's edge points, label anchors, `viewBox`
//!   and `requiredViewBox`, and the rendered edge points (`data-composition-points`) and label masks.
//! - The router's failure goldens (`explicit-pin-conflict`, `pinned-label-over-node`,
//!   `route-preset-conflict`, `viewbox-capacity`): the whole diagnostic.
//! - `tests/workflow_route_cases.json` (`gen-workflow-cases.mjs`): random documents, each
//!   compared on Archify's routes, label anchors, sides and canvas (traced inside a patched copy of
//!   the compiler, right after `validateReadablePinnedGeometry`), or on the router failure it threw.
//!
//! Golden floats are read from the text (see `quote_floats`). Every diff is collected and reported
//! before the test fails.

use std::fs;
use std::path::PathBuf;

use ubiq_archify::gates::workflow_v2;
use ubiq_archify::layout::workflow::readable_build::{Options, ReadableGeometry, build, compile_geometry};
use ubiq_archify::model::workflow::Workflow;
use ubiq_archify::route::workflow::{Acceptor, side_name};
use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// Quote every non-integer number outside a string, so that `serde_json` (whose default float parser
/// is off by one ulp on some shortest round-trip decimals) hands them over as text and [`num`] parses
/// them exactly. Same as `tests/workflow_placement.rs`.
fn quote_floats(text: &str) -> String {
    let (bytes, mut out, mut i, mut in_string) = (text.as_bytes(), String::with_capacity(text.len() + 4096), 0, false);
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
            let end = (i..bytes.len())
                .find(|&j| !matches!(bytes[j], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'))
                .unwrap_or(bytes.len());
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

/// Read a file twice: plainly (documents) and with quoted floats (expectations).
fn load(path: &PathBuf) -> (Value, Value) {
    let text = fs::read_to_string(path).unwrap();
    (serde_json::from_str(&text).unwrap(), serde_json::from_str(&quote_floats(&text)).unwrap())
}

fn num(v: &Value) -> f64 {
    match v {
        Value::String(s) => s.parse().unwrap_or_else(|_| panic!("not a number: {s}")),
        Value::Null => f64::NAN,
        other => other.as_f64().unwrap_or_else(|| panic!("not a number: {other}")),
    }
}

fn points(v: &Value) -> Vec<[f64; 2]> {
    v.as_array().map(|a| a.iter().map(|p| [num(&p[0]), num(&p[1])]).collect()).unwrap_or_default()
}

struct Diffs {
    name: String,
    lines: Vec<String>,
    checked: usize,
}

impl Diffs {
    fn new(name: &str) -> Self {
        Diffs { name: name.to_owned(), lines: Vec::new(), checked: 0 }
    }

    fn num(&mut self, what: &str, got: f64, want: f64) {
        self.checked += 1;
        if got != want && !(got.is_nan() && want.is_nan()) {
            self.lines.push(format!("{what}: got {got} want {want}"));
        }
    }

    fn points(&mut self, what: &str, got: &[[f64; 2]], want: &[[f64; 2]]) {
        self.checked += 1;
        if got != want {
            self.lines.push(format!("{what}: got {got:?} want {want:?}"));
        }
    }

    fn text(&mut self, what: &str, got: &str, want: &str) {
        self.checked += 1;
        if got != want {
            self.lines.push(format!("{what}: got {got:?} want {want:?}"));
        }
    }

    /// Structural JSON equality where the expectation's quoted floats are numbers.
    fn json(&mut self, what: &str, got: &Value, want: &Value) {
        self.checked += 1;
        if !same(got, want) {
            self.lines.push(format!("{what}:\n  got  {got}\n  want {want}"));
        }
    }
}

fn same(got: &Value, want: &Value) -> bool {
    match (got, want) {
        (Value::Number(g), Value::String(w)) => w.parse::<f64>().is_ok_and(|w| g.as_f64() == Some(w)),
        (Value::Number(g), Value::Number(w)) => g.as_f64() == w.as_f64(),
        (Value::Array(g), Value::Array(w)) => g.len() == w.len() && g.iter().zip(w).all(|(a, b)| same(a, b)),
        (Value::Object(g), Value::Object(w)) => g.len() == w.len() && g.iter().all(|(k, v)| w.get(k).is_some_and(|x| same(v, x))),
        _ => got == want,
    }
}

fn doc(v: &Value) -> Workflow {
    serde_json::from_value(v.clone()).expect("a valid workflow")
}

/// Compare a laid-out geometry with Archify's receipt (`edges`, `labels`, `viewBox`).
fn compare_receipt(d: &mut Diffs, g: &ReadableGeometry, receipt: &Value) {
    let w = g.placement.workflow();
    let edges = receipt["edges"].as_array().cloned().unwrap_or_default();
    d.num("edge count", g.edges.len() as f64, edges.len() as f64);
    for (k, want) in edges.iter().enumerate() {
        match g.edge(k) {
            Some(e) => d.points(&format!("edge {k} ({}) points", w.edges[k].id.clone().unwrap_or_default()), &e.points, &points(&want["points"])),
            None => d.lines.push(format!("edge {k}: not routed")),
        }
    }
    let labels = receipt["labels"].as_array().cloned().unwrap_or_default();
    d.num("label count", g.labels.len() as f64, labels.len() as f64);
    for (got, want) in g.labels.iter().zip(&labels) {
        d.text("label text", &got.text, want["label"].as_str().unwrap_or_default());
        d.num(&format!("label {} x", got.text), got.at[0], num(&want["x"]));
        d.num(&format!("label {} y", got.text), got.at[1], num(&want["y"]));
        d.num(&format!("label {} width", got.text), got.rect.width, num(&want["width"]));
    }
    let vb = &receipt["viewBox"];
    let rv = &receipt["requiredViewBox"];
    d.num("viewBox w", g.view_box[0], num(&vb[0]));
    d.num("viewBox h", g.view_box[1], num(&vb[1]));
    d.num("requiredViewBox w", g.required_view_box[0], num(&rv[0]));
    d.num("requiredViewBox h", g.required_view_box[1], num(&rv[1]));
}

/// The rendered edge points and label masks.
fn compare_render(d: &mut Diffs, g: &ReadableGeometry, render: &Value) {
    for (k, want) in render["edges"].as_array().into_iter().flatten().enumerate() {
        if let Some(e) = g.edge(k) {
            d.points(&format!("render edge {k} points"), &e.points, &points(&want["points"]));
        }
    }
    let labels = render["labels"].as_array().cloned().unwrap_or_default();
    for (got, want) in g.labels.iter().zip(&labels) {
        let r = &want["rect"];
        d.num(&format!("render label {} x", got.text), got.rect.x, num(&r["x"]));
        d.num(&format!("render label {} y", got.text), got.rect.y, num(&r["y"]));
        d.num(&format!("render label {} w", got.text), got.rect.width, num(&r["width"]));
    }
    let vb = &render["viewBox"];
    d.num("render viewBox w", g.view_box[0], num(&vb[2]));
    d.num("render viewBox h", g.view_box[1], num(&vb[3]));
}

/// A diagnostic as Archify prints it (no `subject.rule`).
fn diagnostic_json(d: &ubiq_archify::diag::Diagnostic) -> Value {
    let mut v = serde_json::to_value(d).unwrap();
    if let Some(s) = v["subject"].as_object_mut() {
        s.remove("rule");
    }
    if let Some(o) = v.as_object_mut() {
        o.remove("suppresses");
    }
    v
}

const ROUTER_CODES: [&str; 4] =
    ["workflow/explicit-pin-conflict", "workflow/route-preset-conflict", "workflow/viewbox-capacity", "workflow/solver-budget-exhausted"];

fn report(all: Vec<Diffs>, what: &str) {
    let checked: usize = all.iter().map(|d| d.checked).sum();
    let failed: Vec<&Diffs> = all.iter().filter(|d| !d.lines.is_empty()).collect();
    println!("{what}: {} cases, {checked} values, {} with diffs", all.len(), failed.len());
    if !failed.is_empty() {
        let mut out = String::new();
        for d in failed.iter().take(12) {
            out.push_str(&format!("\n== {}\n", d.name));
            for line in d.lines.iter().take(12) {
                out.push_str(&format!("  {line}\n"));
            }
        }
        panic!("{what}: {} of {} differ{out}", failed.len(), all.len());
    }
}

#[test]
fn readable_v2_goldens_route_exactly() {
    let dir = root().join("fixtures");
    let mut names: Vec<String> =
        fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| n.ends_with(".golden.json")).collect();
    names.sort();
    let mut all = Vec::new();
    for name in names {
        let path = dir.join(&name);
        let (plain, quoted) = load(&path);
        if plain["type"] != "workflow" || plain["source_doc"]["schema_version"] != 2 {
            continue;
        }
        let receipt = &quoted["layout_json"]["receipt"];
        if receipt["contract"] != "readable-v2" {
            continue;
        }
        let code = receipt["diagnostics"][0]["code"].as_str().unwrap_or_default().to_owned();
        let laid_out = quoted["layout_json"]["exit_code"] == 0;
        if !laid_out && !ROUTER_CODES.contains(&code.as_str()) {
            continue;
        }
        let mut d = Diffs::new(&name);
        let w = doc(&plain["source_doc"]);
        match build(&w) {
            Ok((scene, g)) if laid_out => {
                compare_receipt(&mut d, &g, receipt);
                compare_render(&mut d, &g, &quoted["render"]);
                for n in &g.placement.nodes {
                    if scene.node(&n.id).is_none() {
                        d.lines.push(format!("scene: no group for node {}", n.id));
                    }
                }
                for e in &g.edges {
                    let edge = &w.edges[e.index];
                    d.num(&format!("scene edges of {}", edge.from), scene.edges_of(&edge.from).len().min(1) as f64, 1.0);
                }
            }
            Ok(_) => d.lines.push(format!("laid out; Archify failed with {code}")),
            Err(diags) if laid_out => d.lines.push(format!("failed: {}", diags.iter().map(|x| x.message.clone()).collect::<Vec<_>>().join(" | "))),
            Err(diags) => {
                let mut want = receipt["diagnostics"][0].clone();
                let mut got = diags.first().map(diagnostic_json).unwrap_or(Value::Null);
                if code == "workflow/viewbox-capacity" {
                    // The golden prints the fix as `[768,404]`; Archify's compiler as `[768, 404]`.
                    for v in [&mut want, &mut got] {
                        if let Some(Value::Array(fixes)) = v.get_mut("supportedFixes") {
                            for f in fixes.iter_mut() {
                                *f = Value::String(f.as_str().unwrap_or_default().replace(", ", ","));
                            }
                        }
                    }
                }
                d.json("diagnostic", &got, &want);
            }
        }
        all.push(d);
    }
    report(all, "readable-v2 goldens");
}

#[test]
fn readable_v2_corpus_routes_exactly() {
    let path = root().join("workflow_route_cases.json");
    if !path.exists() {
        return;
    }
    let (plain, quoted) = load(&path);
    // Fix verification is the full compile, gates included (`acceptsFix`).
    let full = |doc: &Workflow| workflow_v2::compile(doc, workflow_v2::gate_of(doc), false).is_ok();
    let accepts: &Acceptor<'_> = &full;
    let mut all = Vec::new();
    for (case, want) in plain["cases"].as_array().unwrap().iter().zip(quoted["cases"].as_array().unwrap()) {
        let name = case["name"].as_str().unwrap_or_default();
        let mut d = Diffs::new(name);
        let w = doc(&case["doc"]);
        let trace = &want["trace"];
        let failure = &want["failure"];
        // Only a pin conflict's evidence (`conflictingPins`) depends on fix verification.
        let pins = failure["diagnostics"][0]["code"] == "workflow/explicit-pin-conflict";
        match compile_geometry(&w, &Options { accepts: pins.then_some(accepts), gates: None }) {
            Ok(g) if !trace.is_null() => {
                compare_receipt(&mut d, &g, trace);
                for (k, s) in trace["sides"].as_array().into_iter().flatten().enumerate() {
                    if s.is_null() {
                        continue;
                    }
                    let got = g.sides(k).map(|x| format!("{}:{}", side_name(x.from), side_name(x.to))).unwrap_or_default();
                    d.text(&format!("edge {k} sides"), &got, &format!("{}:{}", s[0].as_str().unwrap_or(""), s[1].as_str().unwrap_or("")));
                }
            }
            Ok(_) => d.lines.push(format!("laid out; Archify failed: {}", failure["diagnostics"][0]["code"])),
            Err(f) if !failure.is_null() => {
                let mut want = failure["diagnostics"][0].clone();
                if let Some(o) = want.as_object_mut() {
                    o.remove("supportedFixes");
                }
                let mut got = f.diagnostics.first().map(diagnostic_json).unwrap_or(Value::Null);
                if let Some(o) = got.as_object_mut() {
                    o.remove("supportedFixes");
                }
                d.json("diagnostic", &got, &want);
            }
            Err(f) => d.lines.push(format!("failed: {}", f.error)),
        }
        all.push(d);
    }
    report(all, "readable-v2 corpus");
}
