//! P3.3/P3.4: the architecture router against Archify's own `--layout-json` and render (D5), with no
//! tolerance (`==` on `f64`): every connection's points, its `labelAt` (rounded, as the receipt
//! prints it) and label rect, the rendered path `d`, the crossover halo flags, and the viewBox.
//!
//! Three corpora: the architecture goldens (`tests/fixtures`), `tests/placement_cases.json` (P3.1's
//! random documents, plain automatic connections) and `tests/route_cases.json`
//! (`gen-route-cases.mjs`: fan-outs, reciprocal pairs, authored sides, presets, `via`, pinned
//! and offset labels, widths, boundaries, profiles).
//!
//! `build` includes the showcase label relocation (P3.5); there is no allowance for a differing
//! label: every case must be exact.

use std::fs;
use std::path::PathBuf;

use ubiq_archify::diag::js_round;
use ubiq_archify::geom::{Pt, Rect, rounded_path};
use ubiq_archify::layout::architecture_build::build;
use ubiq_archify::model::architecture::Architecture;
use serde_json::Value;

/// serde_json's default float parser is up to 1 ulp off; every number token becomes `"n:<token>"`
/// and [`num`] parses it with `str::parse`.
fn exact_numbers(text: &str) -> String {
    let token = regex::Regex::new(r#""(?:[^"\\]|\\.)*"|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)"#).unwrap();
    token
        .replace_all(text, |caps: &regex::Captures| match caps.get(1) {
            Some(n) => format!("\"n:{}\"", n.as_str()),
            None => caps[0].to_owned(),
        })
        .into_owned()
}

fn num(v: &Value) -> f64 {
    match v.as_str().and_then(|s| s.strip_prefix("n:")) {
        Some(n) => n.parse().unwrap_or_else(|_| panic!("not a number: {v}")),
        None => v.as_f64().unwrap_or_else(|| panic!("not a number: {v}")),
    }
}

fn pt(v: &Value) -> Pt {
    [num(&v[0]), num(&v[1])]
}

fn points(v: &Value) -> Vec<Pt> {
    v.as_array().unwrap().iter().map(pt).collect()
}

/// What Archify printed for one connection.
struct Expected {
    points: Vec<Pt>,
    /// Rounded.
    label_at: Option<Pt>,
    /// The rendered label: anchor and rect, exact (goldens only).
    label_exact: Option<(Pt, Rect)>,
    /// Rendered `d`, halo and independence flags (goldens only).
    render: Option<(String, bool, bool)>,
}

struct Case {
    name: String,
    doc: Architecture,
    view_box: [f64; 2],
    connections: Vec<Expected>,
    /// Receipt labels `[x, y, width]`, rounded, in order.
    labels: Option<Vec<[f64; 3]>>,
}

fn golden_cases() -> Vec<Case> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut out: Vec<Case> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let path = e.unwrap().path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            if !name.ends_with(".architecture.golden.json") || name.starts_with("neg.") {
                return None;
            }
            let text = fs::read_to_string(&path).unwrap();
            let v: Value = serde_json::from_str(&exact_numbers(&text)).unwrap();
            let receipt = v.pointer("/layout_json/receipt").filter(|r| r.get("connections").is_some())?.clone();
            let render = v.get("render").filter(|r| r.get("edges").is_some())?.clone();
            let plain: Value = serde_json::from_str(&text).unwrap();
            let doc: Architecture = serde_json::from_value(plain["source_doc"].clone()).unwrap();
            let edges = render["edges"].as_array().unwrap();
            let rendered_labels = render["labels"].as_array().unwrap();
            let connections = receipt["connections"]
                .as_array()
                .unwrap()
                .iter()
                .zip(edges)
                .map(|(c, e)| {
                    let key = num(&e["key"]);
                    let label = rendered_labels.iter().find(|l| num(&l["edge_key"]) == key).map(|l| {
                        let r = &l["rect"];
                        (
                            [num(&l["texts"][0]["x"]), num(&l["texts"][0]["y"])],
                            Rect::new(num(&r["x"]), num(&r["y"]), num(&r["width"]), num(&r["height"])),
                        )
                    });
                    Expected {
                        points: points(&c["points"]),
                        label_at: c.get("labelAt").map(pt),
                        label_exact: label,
                        render: Some((
                            e["d"].as_str().unwrap().to_owned(),
                            e.get("crossover").and_then(Value::as_str) == Some("halo"),
                            e.get("independent").and_then(Value::as_bool) == Some(true),
                        )),
                    }
                })
                .collect();
            let labels = receipt["labels"].as_array().unwrap().iter().map(|l| [num(&l["x"]), num(&l["y"]), num(&l["width"])]).collect();
            Some(Case { name, doc, view_box: [num(&receipt["viewBox"][0]), num(&receipt["viewBox"][1])], connections, labels: Some(labels) })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn corpus(file: &str) -> Vec<Case> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join(file);
    let text = fs::read_to_string(&path).unwrap();
    let exact: Value = serde_json::from_str(&exact_numbers(&text)).unwrap();
    let plain: Value = serde_json::from_str(&text).unwrap();
    exact
        .as_array()
        .unwrap()
        .iter()
        .zip(plain.as_array().unwrap())
        .map(|(e, p)| {
            let receipt = &e["receipt"];
            Case {
                name: p["name"].as_str().unwrap().to_owned(),
                doc: serde_json::from_value(p["doc"].clone()).unwrap(),
                view_box: [num(&receipt["viewBox"][0]), num(&receipt["viewBox"][1])],
                connections: receipt["connections"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| Expected {
                        points: points(&c["points"]),
                        label_at: c.get("labelAt").map(pt),
                        label_exact: None,
                        render: None,
                    })
                    .collect(),
                labels: receipt
                    .get("labels")
                    .map(|ls| ls.as_array().unwrap().iter().map(|l| [num(&l["x"]), num(&l["y"]), num(&l["width"])]).collect()),
            }
        })
        .collect()
}

#[derive(Default)]
struct Tally {
    cases: usize,
    exact: usize,
    failed: Vec<String>,
    values: usize,
    /// Planner counters summed: grid searches, grid-routed, crossovers, budget exhausted,
    /// readability improved, reciprocal improved.
    counters: [usize; 6],
}

fn round_pt(p: Pt) -> Pt {
    [js_round(p[0]), js_round(p[1])]
}

fn check(case: &Case, tally: &mut Tally) {
    tally.cases += 1;
    let (scene, g) = match build(&case.doc) {
        Ok(v) => v,
        Err(d) => {
            tally.failed.push(format!("{}: build failed: {:?}", case.name, d.iter().map(|d| &d.message).collect::<Vec<_>>()));
            return;
        }
    };
    let m = &g.metrics;
    for (slot, v) in tally.counters.iter_mut().zip([
        m.grid_search_count,
        m.grid_routed_count,
        m.crossover_routed_count,
        m.grid_budget_exhausted_count,
        m.readability_improved_count,
        m.reciprocal_improved_count,
    ]) {
        *slot += v;
    }
    let mut problems: Vec<String> = Vec::new();
    if g.placement.view_box != case.view_box {
        problems.push(format!("viewBox {:?} != {:?}", g.placement.view_box, case.view_box));
    }
    tally.values += 2;
    if g.connections.len() != case.connections.len() {
        problems.push(format!("{} connections != {}", g.connections.len(), case.connections.len()));
    }
    let edges: Vec<&ubiq_archify::scene::PolylineShape> = scene
        .items
        .iter()
        .filter_map(|i| match &i.shape {
            ubiq_archify::scene::Shape::Polyline(p) => Some(p),
            _ => None,
        })
        .collect();
    for (k, (ours, want)) in g.connections.iter().zip(&case.connections).enumerate() {
        tally.values += want.points.len() * 2;
        if ours.points != want.points {
            problems.push(format!("connection {k} ({}->{}) points\n    ours {:?}\n    want {:?}", ours.from, ours.to, ours.points, want.points));
            continue;
        }
        if let Some((d, halo, independent)) = &want.render {
            tally.values += 3;
            if &rounded_path(&ours.points, 8.0) != d {
                problems.push(format!("connection {k} d {} != {d}", rounded_path(&ours.points, 8.0)));
            }
            if ours.halo != *halo || ours.independent != *independent || edges.get(k).map(|e| e.halo) != Some(*halo) {
                problems.push(format!("connection {k} halo/independent {}/{} != {halo}/{independent}", ours.halo, ours.independent));
            }
        }
        if case.labels.is_none() {
            // placement_cases.json carries no label geometry.
            continue;
        }
        let ours_at = ours.label.as_ref().map(|l| l.at);
        let exact_ok = match (&want.label_exact, ours.label.as_ref()) {
            (Some((want_at, want_rect)), Some(l)) => l.at == *want_at && l.rect == *want_rect,
            (None, _) => true,
            _ => false,
        };
        tally.values += 2;
        if ours_at.map(round_pt) != want.label_at || !exact_ok {
            problems.push(format!("connection {k} labelAt {:?} != {:?}", ours_at.map(round_pt), want.label_at));
        }
    }
    if let Some(want) = &case.labels {
        let ours: Vec<[f64; 3]> = g
            .connections
            .iter()
            .filter_map(|c| c.label.as_ref().map(|l| [js_round(l.rect.x), js_round(l.rect.y), js_round(l.rect.width)]))
            .collect();
        tally.values += want.len() * 3;
        if &ours != want {
            problems.push(format!("labels {ours:?} != {want:?}"));
        }
    }
    if !problems.is_empty() {
        tally.failed.push(format!("{}:\n  {}", case.name, problems.join("\n  ")));
    } else {
        tally.exact += 1;
    }
}

fn run(label: &str, cases: &[Case]) {
    let mut tally = Tally::default();
    for case in cases {
        check(case, &mut tally);
    }
    println!(
        "{label}: {} cases, {} exact, {} failed, ~{} values",
        tally.cases,
        tally.exact,
        tally.failed.len(),
        tally.values
    );
    let [searches, routed, crossovers, exhausted, readable, reciprocal] = tally.counters;
    println!(
        "  planner: {searches} grid searches ({routed} accepted, {crossovers} with a crossover, {exhausted} over budget), \
         {readable} readability improvements, {reciprocal} reciprocal repairs"
    );
    for f in tally.failed.iter().take(12) {
        println!("  FAIL {f}");
    }
    assert!(tally.failed.is_empty(), "{label}: {} of {} cases differ", tally.failed.len(), tally.cases);
}

#[test]
fn architecture_goldens_route_exactly() {
    let cases = golden_cases();
    assert_eq!(cases.len(), 9, "the 9 architecture goldens");
    run("goldens", &cases);
}

#[test]
fn placement_corpus_routes_exactly() {
    run("placement_cases", &corpus("placement_cases.json"));
}

#[test]
fn route_corpus_routes_exactly() {
    run("route_cases", &corpus("route_cases.json"));
}
