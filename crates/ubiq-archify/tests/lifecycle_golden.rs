//! P4.2/P4.3: the lifecycle layout, v2 (grid router) and v1 (fixed bands, the architecture router),
//! against Archify's own render (D5). For every lifecycle golden with a render,
//! `layout::lifecycle::build` must give, with no tolerance (`==` on `f64`): the viewBox, the state
//! rects, text, sigils and initial markers, the band titles (v1: also the dotted rules and the
//! implied rail), the transition points (and their rounded `d`, width, dash and halo), the label
//! rects and text (after the showcase placement), and the legend. Every diff is collected and
//! reported before the test fails.
//!
//! Also here: every rendered case compiles to a lifecycle layout and passes or fails `gates::lifecycle`
//! as Archify's own receipt says at each quality; a v2 document whose pin the grid plan contradicts
//! (the architecture router's v2 case) lays out; the negatives never panic.

use std::fs;
use std::path::PathBuf;

use ubiq_archify::compile::{Layout, Opts, compile};
use ubiq_archify::gates::{Gate, lifecycle as gates};
use ubiq_archify::geom::{polyline_path, rounded_path};
use ubiq_archify::layout::lifecycle::{LifecycleGeometry, Plan, build};
use ubiq_archify::model::{self, Doc};
use ubiq_archify::scene::{Cmd, Group, GroupId, Layer, PolylineShape, Scene, Shape, TextShape};
use ubiq_archify::tokens::Kind;
use serde_json::Value;

struct Case {
    name: String,
    doc: Value,
    render: Value,
    /// The whole golden (the validate receipts live here).
    golden: Value,
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Quote every non-integer number outside a string, so that `serde_json` (whose default float parser
/// can be one ulp off on shortest round-trip decimals) hands them over as text for [`num`].
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

fn lifecycle_files() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = fs::read_dir(fixtures())
        .unwrap()
        .filter_map(|e| {
            let path = e.unwrap().path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            name.ends_with(".golden.json").then(|| (name, fs::read_to_string(&path).unwrap()))
        })
        .filter(|(_, text)| serde_json::from_str::<Value>(text).is_ok_and(|v| v["type"] == "lifecycle"))
        .collect();
    out.sort();
    out
}

/// The cases with a render, v1 and v2.
fn cases() -> Vec<Case> {
    lifecycle_files()
        .into_iter()
        .filter_map(|(name, text)| {
            let v: Value = serde_json::from_str(&text).unwrap();
            let exact: Value = serde_json::from_str(&quote_floats(&text)).unwrap();
            let render = exact.get("render").filter(|r| r.get("nodes").is_some())?.clone();
            Some(Case { name, doc: v["source_doc"].clone(), render, golden: v })
        })
        .collect()
}

/// The generated corpus (`gen-lifecycle-cases.mjs`): random v1 documents and random v2
/// documents with pins, each with Archify's own render.
fn corpus() -> Vec<Case> {
    let text = fs::read_to_string(fixtures().parent().unwrap().join("lifecycle_cases.json")).unwrap();
    let plain: Vec<Value> = serde_json::from_str(&text).unwrap();
    let exact: Vec<Value> = serde_json::from_str(&quote_floats(&text)).unwrap();
    plain
        .into_iter()
        .zip(exact)
        .map(|(p, e)| Case { name: p["name"].as_str().unwrap().to_owned(), doc: p["doc"].clone(), render: e["render"].clone(), golden: Value::Null })
        .collect()
}

fn num(v: &Value) -> f64 {
    match v {
        Value::String(s) => s.parse().unwrap_or_else(|_| panic!("not a number: {s}")),
        other => other.as_f64().unwrap_or_else(|| panic!("not a number: {other}")),
    }
}

fn rect_of(v: &Value) -> [f64; 4] {
    [num(&v["x"]), num(&v["y"]), num(&v["width"]), num(&v["height"])]
}

struct Diffs {
    lines: Vec<String>,
    checked: usize,
}

impl Diffs {
    fn num(&mut self, what: &str, got: f64, want: f64) {
        self.checked += 1;
        if got != want {
            self.lines.push(format!("{what}: got {got} want {want}"));
        }
    }

    fn nums(&mut self, what: &str, got: &[f64], want: &[f64]) {
        if got.len() != want.len() {
            self.lines.push(format!("{what}: got {got:?} want {want:?}"));
            return;
        }
        for (i, (g, w)) in got.iter().zip(want).enumerate() {
            self.num(&format!("{what}[{i}]"), *g, *w);
        }
    }

    fn text(&mut self, what: &str, got: &str, want: &str) {
        self.checked += 1;
        if got != want {
            self.lines.push(format!("{what}: got {got:?} want {want:?}"));
        }
    }

    fn count(&mut self, what: &str, got: usize, want: usize) -> bool {
        self.checked += 1;
        if got != want {
            self.lines.push(format!("{what}: got {got} want {want}"));
        }
        got == want
    }

    fn shape_text(&mut self, what: &str, got: &TextShape, want: &Value) {
        self.text(&format!("{what} text"), &got.text, want["text"].as_str().unwrap_or(""));
        self.num(&format!("{what} x"), got.at[0], num(&want["x"]));
        self.num(&format!("{what} y"), got.at[1], num(&want["y"]));
        self.num(&format!("{what} font_size"), got.size, num(&want["font_size"]));
    }
}

fn gid(scene: &Scene, id: &str) -> Option<GroupId> {
    scene.groups.iter().position(|g| g.id == id).map(|i| GroupId(i as u32))
}

fn group<'a>(scene: &'a Scene, id: &str) -> Option<&'a Group> {
    scene.groups.iter().find(|g| g.id == id)
}

fn texts_of<'a>(scene: &'a Scene, id: &str) -> Vec<&'a TextShape> {
    let id = gid(scene, id).unwrap_or_else(|| panic!("no group {id}"));
    scene
        .items_of(id)
        .filter_map(|i| match &i.shape {
            Shape::Text(t) => Some(t),
            _ => None,
        })
        .collect()
}

fn numbers_in(s: &str) -> Vec<f64> {
    s.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .filter(|p| !p.is_empty())
        .map(|p| p.parse().unwrap())
        .collect()
}

fn dash_of(dash: &[f64]) -> String {
    dash.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(",")
}

/// `M x y L x y ... Z` of a straight-segment path, as Archify writes it.
fn path_d(cmds: &[Cmd]) -> String {
    cmds.iter()
        .map(|c| match c {
            Cmd::M(p) => format!("M {} {}", p[0], p[1]),
            Cmd::L(p) => format!("L {} {}", p[0], p[1]),
            Cmd::Z => "Z".to_owned(),
            other => format!("{other:?}"),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A legend or marker shape against the golden's `{tag, ...}`.
fn compare_shape(d: &mut Diffs, what: &str, got: &Shape, want: &Value) {
    match (got, want["tag"].as_str().unwrap_or("")) {
        (Shape::Text(t), "text") => d.shape_text(what, t, want),
        (Shape::Rect(r), "rect") => d.nums(
            &format!("{what} rect"),
            &[r.rect.x, r.rect.y, r.rect.width, r.rect.height, r.radius],
            &[num(&want["x"]), num(&want["y"]), num(&want["width"]), num(&want["height"]), num(&want["rx"])],
        ),
        (Shape::Path(p), "circle") => {
            // The circle is four arcs from `(cx + r, cy)`.
            let r = num(&want["r"]);
            match p.cmds.first() {
                Some(Cmd::M(start)) => d.nums(&format!("{what} circle"), &[start[0] - r, start[1]], &[num(&want["cx"]), num(&want["cy"])]),
                _ => d.lines.push(format!("{what}: circle path has no start")),
            }
        }
        (Shape::Path(p), "path") => d.text(&format!("{what} path"), &path_d(&p.cmds), want["d"].as_str().unwrap_or("")),
        (Shape::Polyline(p), "path") => d.text(&format!("{what} path"), &polyline_path(&p.points), want["d"].as_str().unwrap_or("")),
        (other, tag) => d.lines.push(format!("{what}: scene {other:?} vs golden {tag}")),
    }
}

fn compare(d: &mut Diffs, case: &Case, scene: &Scene, g: &LifecycleGeometry) {
    let r = &case.render;
    let vb = r["viewBox"].as_array().unwrap();
    d.nums("viewBox", &g.view_box, &[num(&vb[2]), num(&vb[3])]);
    d.nums("scene viewBox", &scene.view_box, &[num(&vb[2]), num(&vb[3])]);

    // states
    let nodes = r["nodes"].as_array().unwrap();
    if d.count("states", g.states.len(), nodes.len()) {
        for (i, n) in nodes.iter().enumerate() {
            let id = n["id"].as_str().unwrap();
            let got = &g.states[i];
            d.text(&format!("state {i} id"), &got.id, id);
            d.nums(&format!("state {id} rect"), &[got.rect.x, got.rect.y, got.rect.width, got.rect.height], &rect_of(&n["rect"]));
            d.text(&format!("state {id} initial marker"), &got.initial_marker.to_string(), &n["initial_marker"].as_bool().unwrap_or(false).to_string());
            let ng = group(scene, &format!("node-{id}")).expect("node group");
            d.text(&format!("state {id} context"), ng.context.as_deref().unwrap_or(""), n["context"].as_str().unwrap());
            let class = n["class"].as_str().unwrap();
            d.text(&format!("state {id} class"), ng.node_kind.map_or("", |k| k.as_str()), Kind::parse(&class[2..]).map_or("", |k| k.as_str()));
            let ts = texts_of(scene, &ng.id);
            let want = n["texts"].as_array().unwrap();
            if d.count(&format!("state {id} texts"), ts.len(), want.len()) {
                for (t, w) in ts.iter().zip(want) {
                    d.shape_text(&format!("state {id}"), t, w);
                }
            }
            if let Some(tr) = n["sigil"]["transform"].as_str() {
                let want = numbers_in(tr);
                // The sigil paths follow the marker shapes; their transform is the one SVG writes.
                let path = scene.items_of(gid(scene, &ng.id).unwrap()).find_map(|i| match &i.shape {
                    Shape::Path(p) if p.transform != [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] => Some(p),
                    _ => None,
                });
                match path {
                    Some(p) => d.nums(&format!("state {id} sigil"), &[p.transform[4], p.transform[5], p.transform[0]], &want),
                    None => d.lines.push(format!("state {id}: no sigil path, want {tr}")),
                }
            }
        }
    }

    // band titles: ungrouped texts in the frame layer; v1 also has a dotted rule under each and
    // the implied rail (the ungrouped solid polyline).
    let frames: Vec<&Shape> = scene.items.iter().filter(|i| i.layer == Layer::Frames && i.group.is_none()).map(|i| &i.shape).collect();
    let titles: Vec<&TextShape> = frames.iter().filter_map(|s| if let Shape::Text(t) = s { Some(t) } else { None }).collect();
    let lines: Vec<&PolylineShape> = frames.iter().filter_map(|s| if let Shape::Polyline(p) = s { Some(p) } else { None }).collect();
    let decor: Vec<Value> = r["decor"]["Lifecycle bands"].as_array().unwrap().clone();
    let want: Vec<&Value> = decor.iter().filter(|w| w["tag"] == "text").collect();
    let want_rules: Vec<&Value> = decor.iter().filter(|w| w["tag"] == "path").collect();
    if d.count("band titles", titles.len(), want.len()) && d.count("bands", g.bands.len(), want.len()) {
        for (i, (t, w)) in titles.iter().zip(&want).enumerate() {
            d.shape_text(&format!("band title {i}"), t, w);
        }
    }
    let rules: Vec<&&PolylineShape> = lines.iter().filter(|p| !p.stroke.dash.is_empty()).collect();
    if d.count("band rules", rules.len(), want_rules.len()) {
        for (i, (p, w)) in rules.iter().zip(&want_rules).enumerate() {
            d.text(&format!("band rule {i} path"), &polyline_path(&p.points), w["d"].as_str().unwrap());
            d.text(&format!("band rule {i} dash"), &dash_of(&p.stroke.dash), w["dash"].as_str().unwrap());
            d.num(&format!("band rule {i} width"), p.stroke.width, 0.8);
        }
    }
    let rails: Vec<&&PolylineShape> = lines.iter().filter(|p| p.stroke.dash.is_empty()).collect();
    let want_rail = r["decor"]["Primary lifecycle rail"].as_array().cloned().unwrap_or_default();
    if d.count("rails", rails.len(), want_rail.len()) {
        for (p, w) in rails.iter().zip(&want_rail) {
            d.text("rail path", &polyline_path(&p.points), w["d"].as_str().unwrap());
            d.num("rail width", p.stroke.width, 2.2);
        }
    }

    // transitions and labels
    let edges = r["edges"].as_array().unwrap();
    if d.count("transitions", g.transitions.len(), edges.len()) {
        for (i, e) in edges.iter().enumerate() {
            let got = &g.transitions[i];
            d.num(&format!("transition {i} key"), got.key as f64, num(&e["key"]));
            let want: Vec<f64> = e["points"].as_array().unwrap().iter().flat_map(|p| [num(&p[0]), num(&p[1])]).collect();
            let have: Vec<f64> = got.points.iter().flat_map(|p| [p[0], p[1]]).collect();
            d.nums(&format!("transition {i} points"), &have, &want);
            d.text(&format!("transition {i} d"), &rounded_path(&got.points, got.radius), e["d"].as_str().unwrap());
            d.num(&format!("transition {i} stroke width"), got.stroke_width, num(&e["stroke_width"]));
            d.text(&format!("transition {i} halo"), &got.planned.to_string(), &(e["crossover"] == "halo").to_string());
            let edge = scene.items_of(gid(scene, &format!("edge-{}", got.key)).unwrap()).map(|i| &i.shape).next();
            if let Some(Shape::Polyline(p)) = edge {
                d.num(&format!("transition {i} scene width"), p.stroke.width, got.stroke_width);
                // `security` and `dashed` dashes are CSS classes, not attributes.
                let (got, want) = (dash_of(&p.stroke.dash), e["dash"].as_str().unwrap_or(""));
                if want.is_empty() && matches!(got.as_str(), "5,5" | "4,4") {
                    d.checked += 1;
                } else {
                    d.text(&format!("transition {i} dash"), &got, want);
                }
            } else {
                d.lines.push(format!("transition {i}: no polyline"));
            }
        }
    }
    let labels = r["labels"].as_array().unwrap();
    let ours: Vec<_> = g.transitions.iter().filter_map(|t| t.label.map(|l| (t.key, l))).collect();
    if d.count("labels", ours.len(), labels.len()) {
        for ((key, label), l) in ours.iter().zip(labels) {
            d.num(&format!("label {key} key"), *key as f64, num(&l["edge_key"]));
            d.nums(&format!("label {key} rect"), &[label.rect.x, label.rect.y, label.rect.width, label.rect.height], &rect_of(&l["rect"]));
            let ts = texts_of(scene, &format!("label-{key}"));
            let want = l["texts"].as_array().unwrap();
            if d.count(&format!("label {key} texts"), ts.len(), want.len()) {
                for (t, w) in ts.iter().zip(want) {
                    d.shape_text(&format!("label {key}"), t, w);
                }
            }
        }
    }

    // legend
    match (&g.legend, r.get("legend").filter(|l| !l.is_null())) {
        (None, None) => {}
        (Some(m), Some(l)) => {
            // The title, the labels (so the widths) and the swatches in the document's locale,
            // `zh-CN` included.
            let title = lifecycle(&case.doc, &case.name).meta.locale().legend_title();
            d.text("legend title", &title, l["title"].as_str().unwrap());
            let on_scene = scene.items.iter().find_map(|i| match &i.shape {
                Shape::Text(t) if i.layer == ubiq_archify::scene::Layer::Legend && i.group.is_none() => Some(t.text.clone()),
                _ => None,
            });
            d.text("legend title in the scene", on_scene.as_deref().unwrap_or(""), l["title"].as_str().unwrap());
            d.text("legend bridge", &m.has_interactive().to_string(), &l["bridge"].as_bool().unwrap().to_string());
            let items = l["items"].as_array().unwrap();
            if d.count("legend items", m.entries.len(), items.len()) {
                for (p, w) in m.entries.iter().zip(items) {
                    let kind = p.entry.kind;
                    d.text("legend kind", kind, w["semantic_kind"].as_str().unwrap());
                    d.num(&format!("legend {kind} x"), p.x, num(&w["x"]));
                    d.num(&format!("legend {kind} baseline"), p.baseline, num(&w["baseline"]));
                    d.num(&format!("legend {kind} width"), p.width, num(&w["width"]));
                    let id = gid(scene, &format!("legend-{kind}")).expect("legend group");
                    let shapes = w["shapes"].as_array().unwrap();
                    let items: Vec<&Shape> = scene.items_of(id).map(|i| &i.shape).collect();
                    if d.count(&format!("legend {kind} shapes"), items.len(), shapes.len()) {
                        for (s, ws) in items.iter().zip(shapes) {
                            compare_shape(d, &format!("legend {kind}"), s, ws);
                        }
                    }
                }
            }
        }
        (got, want) => d.lines.push(format!("legend: got {} want {}", got.is_some(), want.is_some())),
    }
}

fn lifecycle(doc: &Value, name: &str) -> ubiq_archify::model::lifecycle::Lifecycle {
    match model::parse("lifecycle", doc).unwrap_or_else(|e| panic!("{name}: {e}")) {
        Doc::Lifecycle(l) => *l,
        _ => panic!("{name}: not a lifecycle"),
    }
}

#[test]
fn lifecycle_layout_matches_archify() {
    let cases = cases();
    assert!(
        cases.iter().filter(|c| c.golden["schema_version"] == 1).count() >= 5 && cases.iter().any(|c| c.golden["schema_version"] == 2),
        "expected the v1 examples and fixtures and the deployment-release example"
    );
    let mut report = Vec::new();
    let mut checked = 0;
    for case in &cases {
        let doc = lifecycle(&case.doc, &case.name);
        let mut d = Diffs { lines: Vec::new(), checked: 0 };
        match build(&doc) {
            Ok((scene, geometry)) => compare(&mut d, case, &scene, &geometry),
            Err(errs) => d.lines.push(format!("build failed: {errs:?}")),
        }
        checked += d.checked;
        println!("{}: {} values compared, {} diffs", case.name, d.checked, d.lines.len());
        for line in &d.lines {
            report.push(format!("{}: {line}", case.name));
        }
    }
    assert!(checked > 1000, "only {checked} values were compared");
    assert!(report.is_empty(), "{} diffs against Archify:\n{}", report.len(), report.join("\n"));
}

/// The generated corpus, exact like the goldens: random v1 documents (bands, rail, presets, pins,
/// the architecture router, label placement) and random v2 documents, the ones with a pin the grid
/// plan contradicts included (the architecture router lays those out).
#[test]
fn generated_lifecycle_documents_match_archify() {
    let (mut report, mut checked, mut v1, mut conflicts) = (Vec::new(), 0, 0, 0);
    let cases = corpus();
    for case in &cases {
        let doc = lifecycle(&case.doc, &case.name);
        let plan = Plan::new(&doc).unwrap();
        if doc.schema_version == ubiq_archify::model::common::SchemaVersion::V1 {
            v1 += 1;
        } else if !plan.grid {
            conflicts += 1;
        }
        let mut d = Diffs { lines: Vec::new(), checked: 0 };
        match build(&doc) {
            Ok((scene, geometry)) => compare(&mut d, case, &scene, &geometry),
            Err(errs) => d.lines.push(format!("build failed: {errs:?}")),
        }
        checked += d.checked;
        report.extend(d.lines.into_iter().map(|line| format!("{}: {line}", case.name)));
    }
    println!("{} generated cases ({v1} v1, {conflicts} v2 with a contradicted pin), {checked} values compared", cases.len());
    assert!(v1 >= 50 && conflicts >= 10, "only {v1} v1 and {conflicts} pin-conflict documents");
    assert!(report.is_empty(), "{} diffs against Archify:\n{}", report.len(), report.join("\n"));
}

/// The generated validation corpus: accepted and rejected random documents, v1 and v2, authored and
/// under `--quality showcase`. Every diagnostic (code, path, severity, whole message) and `ok`
/// agree with Archify's `validate --json`, the artifact checker's (stage `check`) included.
#[test]
fn generated_validation_matches_archify() {
    use ubiq_archify::diag::Severity;
    use std::collections::BTreeSet;
    let path = fixtures().parent().unwrap().join("lifecycle_validate_cases.json");
    let cases: Vec<Value> = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let key = |code: &str, path: Option<&str>, severity: &str, message: &str| {
        let message = if code == "internal/unclassified" { "" } else { message };
        (code.to_owned(), path.map(str::to_owned), severity.to_owned(), message.to_owned())
    };
    let (mut report, mut runs, mut failing) = (Vec::new(), 0, 0);
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        for (run, quality) in [("authored", None), ("showcase", Some("showcase"))] {
            let want = &case[run];
            runs += 1;
            failing += usize::from(want["ok"] == false);
            let theirs: BTreeSet<_> = want["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| key(d["code"].as_str().unwrap(), d["path"].as_str(), d["severity"].as_str().unwrap(), d["message"].as_str().unwrap()))
                .collect();
            let compiled = compile(&case["doc"].to_string(), &Opts { input: "<input>", doc_type: Some("lifecycle"), quality });
            let ours: BTreeSet<_> = compiled
                .diagnostics()
                .iter()
                .map(|d| {
                    let severity = if d.severity == Severity::Warning { "warning" } else { "error" };
                    key(&d.code, d.subject.path.as_deref(), severity, &d.message)
                })
                .collect();
            if ours != theirs {
                report.push(format!(
                    "{name} [{run}]\n    only in Archify: {:?}\n    only in ours:    {:?}",
                    theirs.difference(&ours).collect::<Vec<_>>(),
                    ours.difference(&theirs).collect::<Vec<_>>()
                ));
            } else if compiled.ok() != want["ok"].as_bool().unwrap() {
                report.push(format!("{name} [{run}]: ok: Archify {}, ours {}", want["ok"], compiled.ok()));
            }
        }
    }
    println!("{} documents, {runs} runs compared ({failing} rejected by Archify), none skipped", cases.len());
    assert!(runs > 400 && failing > 100, "only {runs} runs and {failing} rejections");
    assert!(report.is_empty(), "{} mismatches:\n{}", report.len(), report.join("\n"));
}

/// Every rendered case compiles to a lifecycle layout at every quality, and passes the gates exactly
/// when Archify's own receipt for that run says `ok` (the composition rules are compared message by
/// message in `tests/golden.rs`).
#[test]
fn the_rendered_goldens_compile_to_a_lifecycle_layout_and_pass_the_gates_as_archify_did() {
    for case in cases() {
        for (run, quality) in [("authored", None), ("standard", Some("standard")), ("showcase", Some("showcase"))] {
            let compiled = compile(&case.doc.to_string(), &Opts { input: "x.lifecycle.json", quality, ..Opts::default() });
            let receipt = &case.golden["validate_receipt"][run]["receipt"];
            // Stage `check` is the artifact checker (rounded-corner crossings): it rejects the run
            // here as it did in Archify, and the document stays laid out.
            let want = receipt["ok"].as_bool().unwrap();
            assert_eq!(compiled.ok(), want, "{} ({run}): {:?}", case.name, compiled.diagnostics());
            assert!(matches!(compiled.layout, Some(Layout::Lifecycle { .. })), "{} ({run})", case.name);
            if want {
                assert_eq!(compiled.receipt.composition.as_ref().unwrap()["status"], "pass", "{} ({run})", case.name);
            }
        }
        let doc = lifecycle(&case.doc, &case.name);
        let plan = Plan::new(&doc).unwrap();
        let (outcome, _) = gates::validate(&plan, Gate(None));
        assert!(outcome.ok(), "{}: {:?}", case.name, outcome.problems);
    }
}

#[test]
fn every_lifecycle_document_answers_without_panicking() {
    let (mut v1, mut v2) = (0, 0);
    for (name, text) in lifecycle_files() {
        let v: Value = serde_json::from_str(&text).unwrap();
        let Ok(Doc::Lifecycle(doc)) = model::parse("lifecycle", &v["source_doc"]) else { continue };
        let plan = Plan::new(&doc).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        for gate in [Gate(None), Gate(Some("standard")), Gate(Some("showcase"))] {
            let _ = gates::validate(&plan, gate);
        }
        let _ = build(&doc);
        if v["schema_version"] == 2 {
            v2 += 1;
        } else {
            v1 += 1;
        }
    }
    assert!(v1 >= 8 && v2 >= 8, "only {v1} v1 and {v2} v2 lifecycle documents were built");
}
