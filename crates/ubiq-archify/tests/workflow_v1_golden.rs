//! P6.1: the workflow v1 (`fixed-v1`) layout and gates against Archify's own render and receipts
//! (D5). For every v1 workflow golden with a render, `layout::workflow::legacy::build_plan` must give,
//! with no tolerance (`==` on `f64`): the viewBox, the node rects, text and sigils, the lane,
//! exception-lane and group frames and their titles, the phase headers, the edge points (and `d`,
//! width), the label rects and text, and the legend; and the `--layout-json` receipt's columns,
//! nodes, edges and labels. Every diff is collected and reported before the test fails.
//!
//! Also here: the three v1 negatives (`column-capacity`, `edge-through-node`, the short vertical
//! edge) agree with Archify's receipts diagnostic by diagnostic, both views; the gates pass or fail
//! each rendered case as Archify's receipt says at each quality; and the generated corpora
//! (`gen-workflow-v1-cases.mjs`): random v1 documents rendered by Archify, and random
//! documents validated by Archify, authored and under `--quality showcase`.
//!
//! Every plan here is [`LegacyPlan::archify`], Archify's unpadded geometry: the frame padding of
//! `LegacyPlan::new` is a deliberate deviation (D40), recorded in `tests/deviations/` instead.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use ubiq_archify::gates::{Gate, workflow_v1 as gates};
use ubiq_archify::geom::polyline_path;
use ubiq_archify::layout::workflow::legacy::{LegacyGeometry, LegacyPlan, build_plan, padding_changes};
use ubiq_archify::model::workflow::Workflow;
use ubiq_archify::model::{self, Doc};
use ubiq_archify::scene::{Cmd, Group, GroupId, Layer, Scene, Shape, TextShape};
use ubiq_archify::tokens::Kind;
use serde_json::Value;

struct Case {
    name: String,
    doc: Value,
    render: Value,
    /// The whole golden (the receipts live here); `Null` for the generated corpus.
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

fn workflow_files() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = fs::read_dir(fixtures())
        .unwrap()
        .filter_map(|e| {
            let path = e.unwrap().path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            name.ends_with(".golden.json").then(|| (name, fs::read_to_string(&path).unwrap()))
        })
        .filter(|(_, text)| serde_json::from_str::<Value>(text).is_ok_and(|v| v["type"] == "workflow"))
        .collect();
    out.sort();
    out
}

/// The v1 cases with a render.
fn cases() -> Vec<Case> {
    workflow_files()
        .into_iter()
        .filter_map(|(name, text)| {
            let v: Value = serde_json::from_str(&text).unwrap();
            if v["schema_version"] != 1 {
                return None;
            }
            let exact: Value = serde_json::from_str(&quote_floats(&text)).unwrap();
            let render = exact.get("render").filter(|r| r.get("nodes").is_some())?.clone();
            Some(Case { name, doc: v["source_doc"].clone(), render, golden: exact })
        })
        .collect()
}

/// The generated corpus: random v1 documents with Archify's own render.
fn corpus() -> Vec<Case> {
    let text = fs::read_to_string(fixtures().parent().unwrap().join("workflow_v1_cases.json")).unwrap();
    let plain: Vec<Value> = serde_json::from_str(&text).unwrap();
    let exact: Vec<Value> = serde_json::from_str(&quote_floats(&text)).unwrap();
    plain
        .into_iter()
        .zip(exact)
        .map(|(p, e)| Case { name: p["name"].as_str().unwrap().to_owned(), doc: p["doc"].clone(), render: e["render"].clone(), golden: e })
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
    fn new() -> Self {
        Diffs { lines: Vec::new(), checked: 0 }
    }

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

/// A legend shape against the golden's `{tag, ...}`.
fn compare_shape(d: &mut Diffs, what: &str, got: &Shape, want: &Value) {
    match (got, want["tag"].as_str().unwrap_or("")) {
        (Shape::Text(t), "text") => d.shape_text(what, t, want),
        (Shape::Rect(r), "rect") => d.nums(
            &format!("{what} rect"),
            &[r.rect.x, r.rect.y, r.rect.width, r.rect.height, r.radius],
            &[num(&want["x"]), num(&want["y"]), num(&want["width"]), num(&want["height"]), num(&want["rx"])],
        ),
        (Shape::Path(p), "path") => d.text(&format!("{what} path"), &path_d(&p.cmds), want["d"].as_str().unwrap_or("")),
        (other, tag) => d.lines.push(format!("{what}: scene {other:?} vs golden {tag}")),
    }
}

fn workflow(doc: &Value, name: &str) -> Workflow {
    match model::parse("workflow", doc).unwrap_or_else(|e| panic!("{name}: {e}")) {
        Doc::Workflow(w) => *w,
        _ => panic!("{name}: not a workflow"),
    }
}

fn compare(d: &mut Diffs, case: &Case, plan: &LegacyPlan<'_>, scene: &Scene, g: &LegacyGeometry) {
    let r = &case.render;
    let vb = r["viewBox"].as_array().unwrap();
    d.nums("viewBox", &g.view_box, &[num(&vb[2]), num(&vb[3])]);
    d.nums("scene viewBox", &scene.view_box, &[num(&vb[2]), num(&vb[3])]);

    // nodes
    let nodes = r["nodes"].as_array().unwrap();
    if d.count("nodes", g.nodes.len(), nodes.len()) {
        for (i, n) in nodes.iter().enumerate() {
            let id = n["id"].as_str().unwrap();
            let got = &g.nodes[i];
            d.text(&format!("node {i} id"), &got.id, id);
            d.nums(&format!("node {id} rect"), &[got.rect.x, got.rect.y, got.rect.width, got.rect.height], &rect_of(&n["rect"]));
            d.num(&format!("node {id} rx"), 6.0, num(&n["rect"]["rx"]));
            let ng = group(scene, &format!("node-{id}")).expect("node group");
            d.text(&format!("node {id} context"), ng.context.as_deref().unwrap_or(""), n["context"].as_str().unwrap());
            let class = n["class"].as_str().unwrap();
            d.text(&format!("node {id} class"), ng.node_kind.map_or("", |k| k.as_str()), Kind::parse(&class[2..]).map_or("", |k| k.as_str()));
            let ts = texts_of(scene, &ng.id);
            let want = n["texts"].as_array().unwrap();
            if d.count(&format!("node {id} texts"), ts.len(), want.len()) {
                for (t, w) in ts.iter().zip(want) {
                    d.shape_text(&format!("node {id}"), t, w);
                }
            }
            if let Some(tr) = n["sigil"]["transform"].as_str() {
                let want = numbers_in(tr);
                let path = scene.items_of(gid(scene, &ng.id).unwrap()).find_map(|i| match &i.shape {
                    Shape::Path(p) if p.transform != [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] => Some(p),
                    _ => None,
                });
                match path {
                    Some(p) => d.nums(&format!("node {id} sigil"), &[p.transform[4], p.transform[5], p.transform[0]], &want),
                    None => d.lines.push(format!("node {id}: no sigil path, want {tr}")),
                }
            }
        }
    }

    // frames: lanes, exception lanes and groups, as `data-composition-frame-*`
    let frames = plan.frames();
    let want_frames = r["frames"].as_array().unwrap();
    if d.count("frames", frames.len(), want_frames.len()) {
        for (f, w) in frames.iter().zip(want_frames) {
            d.text(&format!("frame {} kind", f.id), f.kind, w["kind"].as_str().unwrap());
            d.text(&format!("frame {} id", f.id), &f.id, w["id"].as_str().unwrap());
            d.nums(&format!("frame {} rect", f.id), &[f.rect.x, f.rect.y, f.rect.width, f.rect.height, f.radius], &{
                let r = rect_of(&w["rect"]);
                [r[0], r[1], r[2], r[3], num(&w["rect"]["rx"])]
            });
        }
    }

    // decor: lane titles, phase headers, group titles
    let lane_titles: Vec<&Value> = r["decor"]["Swimlanes"].as_array().map(|a| a.iter().filter(|w| w["tag"] == "text").collect()).unwrap_or_default();
    if d.count("lane titles", plan.doc.lanes.len(), lane_titles.len()) {
        for (i, w) in lane_titles.iter().enumerate() {
            let ts = texts_of(scene, &format!("frame-lane-{i}"));
            if d.count(&format!("lane {i} texts"), ts.len(), 1) {
                d.shape_text(&format!("lane {i} title"), ts[0], w);
            }
        }
    }
    let ungrouped: Vec<&Shape> = scene.items.iter().filter(|i| i.layer == Layer::Frames && i.group.is_none()).map(|i| &i.shape).collect();
    let want_phase = r["decor"]["Phase headers"].as_array().cloned().unwrap_or_default();
    if d.count("phase header shapes", ungrouped.len(), want_phase.len()) {
        for (i, (s, w)) in ungrouped.iter().zip(&want_phase).enumerate() {
            match (s, w["tag"].as_str().unwrap()) {
                (Shape::Polyline(p), "line") => {
                    d.nums(
                        &format!("phase shape {i} line"),
                        &[p.points[0][0], p.points[0][1], p.points[1][0], p.points[1][1]],
                        &[num(&w["x1"]), num(&w["y1"]), num(&w["x2"]), num(&w["y2"])],
                    );
                    d.num(&format!("phase shape {i} width"), p.stroke.width, 1.1);
                }
                (Shape::Rect(rect), "rect") => d.nums(
                    &format!("phase shape {i} mask"),
                    &[rect.rect.x, rect.rect.y, rect.rect.width, rect.rect.height, rect.radius],
                    &[num(&w["x"]), num(&w["y"]), num(&w["width"]), num(&w["height"]), num(&w["rx"])],
                ),
                (Shape::Text(t), "text") => d.shape_text(&format!("phase shape {i}"), t, w),
                (other, tag) => d.lines.push(format!("phase shape {i}: scene {other:?} vs golden {tag}")),
            }
        }
    }
    let want_groups = r["decor"]["Workflow groups"].as_array().cloned().unwrap_or_default();
    let group_count = plan.doc.groups.as_ref().map_or(0, Vec::len);
    if d.count("group titles", group_count, want_groups.len()) {
        for (i, w) in want_groups.iter().enumerate() {
            let ts = texts_of(scene, &format!("frame-group-{i}"));
            if d.count(&format!("group {i} texts"), ts.len(), 1) {
                d.shape_text(&format!("group {i} title"), ts[0], w);
            }
        }
    }

    // edges and labels
    let edges = r["edges"].as_array().unwrap();
    if d.count("edges", g.edges.len(), edges.len()) {
        for (i, e) in edges.iter().enumerate() {
            let got = &g.edges[i];
            d.num(&format!("edge {i} key"), got.key as f64, num(&e["key"]));
            let want: Vec<f64> = e["points"].as_array().unwrap().iter().flat_map(|p| [num(&p[0]), num(&p[1])]).collect();
            let have: Vec<f64> = got.points.iter().flat_map(|p| [p[0], p[1]]).collect();
            d.nums(&format!("edge {i} points"), &have, &want);
            d.text(&format!("edge {i} d"), &polyline_path(&got.points), e["d"].as_str().unwrap());
            d.num(&format!("edge {i} stroke width"), got.width, num(&e["stroke_width"]));
            let shape = scene.items_of(gid(scene, &format!("edge-{}", got.key)).unwrap()).map(|i| &i.shape).next();
            if let Some(Shape::Polyline(p)) = shape {
                d.num(&format!("edge {i} scene width"), p.stroke.width, got.width);
                let variant = e["variant"].as_str().unwrap();
                d.text(&format!("edge {i} marker"), &format!("{:?}", p.marker.map(|m| m.fill)), &format!("{:?}", marker_for(variant)));
            } else {
                d.lines.push(format!("edge {i}: no polyline"));
            }
        }
    }
    let labels = r["labels"].as_array().unwrap();
    let ours: Vec<_> = g.edges.iter().filter_map(|e| e.label.as_ref().map(|l| (e.key, l))).collect();
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
            // The title, labels and widths in the document's locale, `zh-CN` included.
            let title = model::parse("workflow", &case.doc).unwrap().locale().legend_title();
            d.text("legend title", &title, l["title"].as_str().unwrap());
            d.text("legend bridge", &m.has_interactive().to_string(), &l["bridge"].as_bool().unwrap().to_string());
            let items = l["items"].as_array().unwrap();
            if d.count("legend items", m.entries.len(), items.len()) {
                for (p, w) in m.entries.iter().zip(items) {
                    let kind = p.entry.kind;
                    d.text("legend kind", kind, w["semantic_kind"].as_str().unwrap());
                    d.text(&format!("legend {kind} label"), &p.entry.label, w["label"].as_str().unwrap_or(&p.entry.label));
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

/// The arrowhead fill token of a variant name, as `Debug` prints the scene's marker.
fn marker_for(variant: &str) -> Option<ubiq_archify::tokens::Token> {
    Some(ubiq_archify::tokens::EdgeVariant::parse(Some(variant)).stroke())
}

/// The `--layout-json` receipt of a compiled v1 workflow against the golden's.
fn compare_receipt(d: &mut Diffs, receipt: &Value, g: &LegacyGeometry) {
    d.text("receipt contract", "fixed-v1", receipt["contract"].as_str().unwrap());
    d.nums("receipt viewBox", &g.view_box, &receipt["viewBox"].as_array().unwrap().iter().map(num).collect::<Vec<_>>());
    d.nums("receipt requiredViewBox", &g.required_view_box, &receipt["requiredViewBox"].as_array().unwrap().iter().map(num).collect::<Vec<_>>());
    d.nums("receipt columns", &g.columns, &receipt["columns"].as_array().unwrap().iter().map(num).collect::<Vec<_>>());
    let nodes = receipt["nodes"].as_array().unwrap();
    if d.count("receipt nodes", g.nodes.len(), nodes.len()) {
        for (n, w) in g.nodes.iter().zip(nodes) {
            d.text("receipt node id", &n.id, w["id"].as_str().unwrap());
            d.text(&format!("receipt node {} lane", n.id), &n.lane, w["lane"].as_str().unwrap());
            d.num(&format!("receipt node {} col", n.id), n.col, num(&w["col"]));
            d.nums(&format!("receipt node {}", n.id), &[n.rect.x, n.rect.y, n.rect.width, n.rect.height], &rect_of(w));
        }
    }
    let edges = receipt["edges"].as_array().unwrap();
    if d.count("receipt edges", g.edges.len(), edges.len()) {
        for (e, w) in g.edges.iter().zip(edges) {
            d.text("receipt edge id", e.id.as_deref().unwrap_or("<null>"), w["id"].as_str().unwrap_or("<null>"));
            d.text("receipt edge from", &e.from, w["from"].as_str().unwrap());
            d.text("receipt edge to", &e.to, w["to"].as_str().unwrap());
            let want: Vec<f64> = w["points"].as_array().unwrap().iter().flat_map(|p| [num(&p[0]), num(&p[1])]).collect();
            let have: Vec<f64> = e.points.iter().flat_map(|p| [p[0], p[1]]).collect();
            d.nums(&format!("receipt edge {} points", e.key), &have, &want);
        }
    }
    let labels = receipt["labels"].as_array().unwrap();
    let ours: Vec<_> = g.edges.iter().filter_map(|e| e.label.as_ref().map(|l| (e, l))).collect();
    if d.count("receipt labels", ours.len(), labels.len()) {
        for ((e, l), w) in ours.iter().zip(labels) {
            d.text("receipt label edge", e.id.as_deref().unwrap_or("<null>"), w["edge"].as_str().unwrap_or("<null>"));
            d.text("receipt label text", &l.text, w["label"].as_str().unwrap());
            d.nums(
                &format!("receipt label {}", e.key),
                &[l.at[0], l.at[1], l.rect.width, l.rect.height],
                &[num(&w["x"]), num(&w["y"]), num(&w["width"]), num(&w["height"])],
            );
        }
    }
    d.count("receipt diagnostics", 0, receipt["diagnostics"].as_array().map_or(0, Vec::len));
}

#[test]
fn workflow_v1_layout_matches_archify() {
    let cases = cases();
    assert!(cases.len() >= 6, "expected the v1 examples and fixtures, found {}", cases.len());
    let mut report = Vec::new();
    let mut checked = 0;
    for case in &cases {
        let doc = workflow(&case.doc, &case.name);
        let plan = LegacyPlan::archify(&doc);
        let mut d = Diffs::new();
        match build_plan(&plan) {
            Ok((scene, geometry)) => {
                compare(&mut d, case, &plan, &scene, &geometry);
                compare_receipt(&mut d, &case.golden["layout_json"]["receipt"], &geometry);
            }
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

/// The generated corpus, exact like the goldens: random v1 documents (lanes, phases, groups, route
/// presets, pins, the one-bend route, port spread, labels, legends, explicit canvases).
#[test]
fn generated_workflow_v1_documents_match_archify() {
    let (mut report, mut checked) = (Vec::new(), 0);
    let cases = corpus();
    for case in &cases {
        let doc = workflow(&case.doc, &case.name);
        let plan = LegacyPlan::archify(&doc);
        let mut d = Diffs::new();
        match build_plan(&plan) {
            Ok((scene, geometry)) => {
                compare(&mut d, case, &plan, &scene, &geometry);
                if !case.golden["layout_json"].is_null() {
                    compare_receipt(&mut d, &case.golden["layout_json"], &geometry);
                }
            }
            Err(errs) => d.lines.push(format!("build failed: {errs:?}")),
        }
        checked += d.checked;
        report.extend(d.lines.into_iter().map(|line| format!("{}: {line}", case.name)));
    }
    println!("{} generated cases, {checked} values compared", cases.len());
    assert!(cases.len() >= 100 && checked > 20_000, "only {} cases and {checked} values", cases.len());
    assert!(report.is_empty(), "{} diffs against Archify:\n{}", report.len(), report.join("\n"));
}

/// The gate a run's quality means: `--quality`, else the document's own `meta.quality_profile`.
fn gate_of(doc: &Workflow, quality: Option<&'static str>) -> Gate {
    use ubiq_archify::model::common::QualityProfile;
    Gate(quality.or(match doc.meta.quality_profile {
        Some(QualityProfile::Standard) => Some("standard"),
        Some(QualityProfile::Showcase) => Some("showcase"),
        None => None,
    }))
}

/// `(code, severity, message)` of a diagnostic, for set comparison.
fn key(code: &str, severity: &str, message: &str) -> (String, String, String) {
    (code.to_owned(), severity.to_owned(), message.to_owned())
}

fn ours(diagnostics: &[ubiq_archify::diag::Diagnostic]) -> BTreeSet<(String, String, String)> {
    diagnostics
        .iter()
        .map(|d| {
            let severity = if d.severity == ubiq_archify::diag::Severity::Warning { "warning" } else { "error" };
            key(&d.code, severity, &d.message)
        })
        .collect()
}

fn theirs(list: &Value) -> BTreeSet<(String, String, String)> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|d| key(d["code"].as_str().unwrap(), d["severity"].as_str().unwrap(), d["message"].as_str().unwrap()))
        .collect()
}

/// Every rendered case passes the gates at every quality when Archify's receipt for that run says
/// `ok`, and the composition status agrees.
#[test]
fn the_rendered_goldens_pass_the_gates_as_archify_did() {
    for case in cases() {
        let doc = workflow(&case.doc, &case.name);
        let plan = LegacyPlan::archify(&doc);
        for (run, quality) in [("authored", None), ("standard", Some("standard")), ("showcase", Some("showcase"))] {
            let receipt = &case.golden["validate_receipt"][run]["receipt"];
            let gate = gate_of(&doc, quality);
            let (outcome, review) = gates::validate_v1(&plan, gate);
            // Stage `check` is the artifact checker, after the gates: they pass here, and the checker
            // is compared on its own (`tests/golden.rs`).
            let want_ok = receipt["ok"].as_bool().unwrap() || receipt["stage"] == "check";
            assert_eq!(outcome.ok(), want_ok, "{} ({run}): {:?}", case.name, outcome.problems);
            if receipt["ok"].as_bool().unwrap() {
                let reported = quality.or(gate.0).unwrap_or("standard");
                let block = gates::receipt(&review, reported, gate);
                assert_eq!(block["status"], receipt["composition"]["status"], "{} ({run}) composition status", case.name);
            }
        }
    }
}

/// The fixture writer (`gen-fixtures.mjs`, `stringify`) collapses `[1, 2]` to `[1,2]` in
/// *every* numeric array of the JSON text, strings included. Apply the same to our side so a message
/// like `... segment 0 [134, 119] -> [254, 119] ...` compares equal to its golden.
fn compact_numeric_arrays(text: &str) -> String {
    use regex::Regex;
    let num = r"-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?";
    let re = Regex::new(&format!(r"\[\s*({num}(?:\s*,\s*{num})*)\s*\]")).unwrap();
    let comma = Regex::new(r"\s*,\s*").unwrap();
    re.replace_all(text, |caps: &regex::Captures<'_>| format!("[{}]", comma.replace_all(&caps[1], ","))).into_owned()
}

/// The three v1 negatives: both views of the diagnostics equal Archify's, field by field (the wire
/// `rule` is ours, not Archify's, and is dropped).
#[test]
fn the_v1_negatives_agree_with_archify() {
    let mut checked = 0;
    for name in ["neg.wf-v1-column-capacity", "neg.wf-v1-edge-through-node", "neg.wf-v1-short-edge-vertical"] {
        let text = fs::read_to_string(fixtures().join(format!("{name}.golden.json"))).unwrap();
        let golden: Value = serde_json::from_str(&text).unwrap();
        let doc = workflow(&golden["source_doc"], name);
        let plan = LegacyPlan::archify(&doc);
        let (outcome, review) = gates::validate_v1(&plan, gate_of(&doc, None));
        assert!(!outcome.ok(), "{name} must be rejected");

        let strip = |d: &ubiq_archify::diag::Diagnostic| {
            let mut v = serde_json::to_value(d).unwrap();
            v["subject"].as_object_mut().unwrap().remove("rule");
            let message = compact_numeric_arrays(v["message"].as_str().unwrap());
            v["message"] = Value::String(message);
            v
        };
        let want = &golden["validate_receipt"]["authored"]["receipt"];
        let got: Vec<Value> = outcome.diagnostics.iter().map(strip).collect();
        assert_eq!(Value::Array(got), want["diagnostics"], "{name}: validate --json diagnostics");
        let expected_error = want["error"].as_str().unwrap();
        let error = review.error.clone().unwrap_or_else(|| format!("Workflow layout validation failed:\n- {}", outcome.problems.join("\n- ")));
        assert_eq!(compact_numeric_arrays(&error), expected_error, "{name}: error text");

        let compiler: Vec<Value> = review.compiler.iter().map(strip).collect();
        assert_eq!(Value::Array(compiler), golden["layout_json"]["receipt"]["diagnostics"], "{name}: layout-json diagnostics");
        checked += 1;
    }
    assert_eq!(checked, 3);
}

/// The generated validation corpus: accepted and rejected random v1 documents, authored and under
/// `--quality showcase`. Every diagnostic (code, severity, whole message) and `ok` agree with
/// Archify's `validate --json`. A run Archify passed (render and artifact checker) also compiles
/// clean here: the checker (`gates::artifact`) does not reject what Archify's accepted. (The corpus
/// has no stage `check` rejection; the checker's own comparison is `tests/golden.rs` and
/// `tests/check_cases.rs`.)
#[test]
fn generated_validation_matches_archify() {
    let path = fixtures().parent().unwrap().join("workflow_v1_validate_cases.json");
    let cases: Vec<Value> = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let (mut report, mut runs, mut failing, mut skipped, mut fixes) = (Vec::new(), 0, 0, 0, 0);
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let doc = workflow(&case["doc"], name);
        let plan = LegacyPlan::archify(&doc);
        for (run, quality) in [("authored", None), ("showcase", Some("showcase"))] {
            let want = &case[run];
            if want["stage"] != "render" && want["ok"] == false {
                // Rejected before the layout (schema, graph, semantic contract): not this pass.
                skipped += 1;
                continue;
            }
            runs += 1;
            failing += usize::from(want["ok"] == false);
            // The compile check runs the padded layout (D40): only where padding changes nothing.
            if want["ok"] == true && !padding_changes(&doc) {
                let opts = ubiq_archify::compile::Opts { input: "<input>", doc_type: Some("workflow"), quality };
                let compiled = ubiq_archify::compile::compile(&case["doc"].to_string(), &opts);
                if !compiled.ok() {
                    report.push(format!("{name} [{run}]: Archify passed it, we reject it: {:?}", compiled.diagnostics()));
                }
            }
            let (outcome, _) = gates::validate_v1(&plan, gate_of(&doc, quality));
            let mut got = ours(&outcome.diagnostics);
            // The legend error surfaces at render time, once the validation passed.
            if outcome.ok()
                && let Err(errs) = build_plan(&plan)
            {
                got = ours(&errs);
            }
            let ok = got.iter().all(|(_, severity, _)| severity == "warning");
            let want_set = theirs(&want["diagnostics"]);
            let want_errors: BTreeSet<_> = want_set.iter().filter(|(_, s, _)| s == "error").cloned().collect();
            let got_errors: BTreeSet<_> = got.iter().filter(|(_, s, _)| s == "error").cloned().collect();
            if got_errors != want_errors {
                report.push(format!(
                    "{name} [{run}]\n    only in Archify: {:?}\n    only in ours:    {:?}",
                    want_errors.difference(&got_errors).collect::<Vec<_>>(),
                    got_errors.difference(&want_errors).collect::<Vec<_>>()
                ));
            } else if ok != want["ok"].as_bool().unwrap() {
                report.push(format!("{name} [{run}]: ok: Archify {}, ours {ok}", want["ok"]));
            } else if let Some(theirs) = want["diagnostics"].as_array().and_then(|l| l.iter().find(|d| d["code"] == "workflow/column-capacity")) {
                // The verified fixes, "migrate to schema_version 2" included (the v2 compile of the migrated document).
                let want_fixes: Vec<&str> = theirs["supportedFixes"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
                let got_fixes: Vec<&str> = outcome
                    .diagnostics
                    .iter()
                    .find(|d| d.code == "workflow/column-capacity")
                    .map(|d| d.supported_fixes.iter().map(String::as_str).collect())
                    .unwrap_or_default();
                fixes += 1;
                if want_fixes != got_fixes {
                    report.push(format!("{name} [{run}]: column-capacity fixes: Archify {want_fixes:?}, ours {got_fixes:?}"));
                }
            }
        }
    }
    println!("{} documents, {runs} runs compared ({failing} rejected by Archify, {fixes} with verified fixes), {skipped} skipped", cases.len());
    assert!(runs > 150 && failing > 40, "only {runs} runs and {failing} rejections");
    assert!(report.is_empty(), "{} mismatches:\n{}", report.len(), report.join("\n"));
}

#[test]
fn every_v1_workflow_document_answers_without_panicking() {
    let mut v1 = 0;
    for (name, text) in workflow_files() {
        let v: Value = serde_json::from_str(&text).unwrap();
        if v["schema_version"] != 1 {
            continue;
        }
        let Ok(Doc::Workflow(doc)) = model::parse("workflow", &v["source_doc"]) else { continue };
        let plan = LegacyPlan::archify(&doc);
        for gate in [Gate(None), Gate(Some("standard")), Gate(Some("showcase"))] {
            let _ = gates::validate_v1(&plan, gate);
        }
        let _ = build_plan(&plan);
        let _ = name;
        v1 += 1;
    }
    assert!(v1 >= 10, "only {v1} v1 workflow documents were built");
}
